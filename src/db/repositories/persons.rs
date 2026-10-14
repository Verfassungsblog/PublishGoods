//! `persons` / `biographies`.
//!
//! [`vb_exchange::projects::Person`]/[`Biography`] are defined in an external crate
//! (`vb-exchange`), which implements `sqlx::FromRow` for both directly (gated behind its
//! `sqlx` feature) — so reads here use `sqlx::query_as` straight into the domain structs,
//! no named intermediate row type. `Person::bios` isn't a plain column (biographies live in
//! their own table), so `FromRow` always leaves it `None`; callers that need it fetch
//! separately via [`fetch_bios`] and fill the field in afterward. Because `query_as` (not the
//! compile-time-checked `query_as!` macro) is what consumes a `FromRow` impl, these read
//! queries are runtime-checked rather than macro-checked.
//!
//! Every person is scoped: it either belongs to a team's shared library (`owner_team_id`) or to
//! a single project (`project_id`), exactly one of the two (`person_scope` check). The scope is
//! part of [`Person`] (`Person::scope`), so every person query selects both columns. Free-text
//! names (`persons_projects.name` / `persons_sections.name`) are not persons and have no scope.

use super::DbError;
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use std::collections::HashSet;
use uuid::Uuid;
use vb_exchange::projects::{Biography, Person, PersonScope};

/// Loads all biographies for a person from the separate `biographies` table.
async fn fetch_bios<'e>(
    exec: impl PgExecutor<'e>,
    person_id: Uuid,
) -> Result<Vec<Biography>, DbError> {
    let bios: Vec<Biography> =
        sqlx::query_as("SELECT content, language FROM biographies WHERE person_id = $1")
            .bind(person_id)
            .fetch_all(exec)
            .await?;
    Ok(bios)
}

/// Replaces all of a person's biographies with `bios` (delete-then-insert) within an
/// existing transaction/connection.
async fn replace_bios<'e>(
    exec: &mut sqlx::PgConnection,
    person_id: Uuid,
    bios: &[Biography],
) -> Result<(), DbError> {
    sqlx::query!("DELETE FROM biographies WHERE person_id = $1", person_id)
        .execute(&mut *exec)
        .await?;

    for bio in bios {
        let language = bio
            .lang
            .map(|l| l.as_tag().to_string())
            .unwrap_or_else(|| "en-US".to_string());
        sqlx::query!(
            "INSERT INTO biographies (person_id, content, language) VALUES ($1, $2, $3)",
            person_id,
            bio.content,
            language
        )
        .execute(&mut *exec)
        .await?;
    }
    Ok(())
}

/// Columns every [`Person`] query must select (`FromRow` derives `scope` from the last two).
const PERSON_COLUMNS: &str =
    "id, first_names, last_names, orcid, gnd, ror, owner_team_id, project_id";

/// Restricts a person search to one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFilter {
    /// A team's shared library only.
    Team(Uuid),
    /// The project's own persons plus the library of the team owning the project.
    Project(Uuid),
}

/// A place a person is linked from: a project's authors/editors, or one of its sections'.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PersonUsage {
    pub project_id: Uuid,
    pub project_title: String,
    pub section_id: Option<Uuid>,
    pub section_title: Option<String>,
    pub role: String,
}

/// Fetches a person by id, including their biographies.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Person, DbError> {
    let mut person: Person = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PERSON_COLUMNS} FROM persons WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound("person"))?;

    person.bios = Some(fetch_bios(pool, id).await?);
    Ok(person)
}

/// Checks whether a person with the given id exists.
pub async fn exists<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<bool, DbError> {
    let exists = sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM persons WHERE id = $1)", id)
        .fetch_one(exec)
        .await?;
    Ok(exists.unwrap_or(false))
}

/// Inserts `person` (which must already have `id` assigned by the caller, matching today's
/// API-level id-assignment convention, and a `scope`) plus its biographies, in one transaction.
pub async fn insert(pool: &PgPool, person: &Person) -> Result<(), DbError> {
    let id = person
        .id
        .ok_or(DbError::Conflict("person id must be assigned".to_string()))?;
    let (owner_team_id, project_id) = match person.scope {
        Some(PersonScope::Team(team_id)) => (Some(team_id), None),
        Some(PersonScope::Project(project_id)) => (None, Some(project_id)),
        None => return Err(DbError::Conflict("person scope must be set".to_string())),
    };
    let mut tx = pool.begin().await?;

    sqlx::query!(
        "INSERT INTO persons (id, first_names, last_names, orcid, gnd, ror, owner_team_id, project_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        id,
        person.first_names,
        person.last_names,
        person.orcid.as_ref().map(|i| &i.value),
        person.gnd.as_ref().map(|i| &i.value),
        person.ror.as_ref().map(|i| &i.value),
        owner_team_id,
        project_id,
    )
    .execute(&mut *tx)
    .await?;

    replace_bios(&mut tx, id, person.bios.as_deref().unwrap_or(&[])).await?;

    tx.commit().await?;
    Ok(())
}

/// Updates `person`'s core fields and replaces its biographies, in one transaction.
/// Fails with [`DbError::NotFound`] if no row with that id exists.
///
/// `person.scope` is deliberately ignored: a person's scope only ever changes through
/// [`promote_to_team`].
pub async fn update(pool: &PgPool, person: &Person) -> Result<(), DbError> {
    let id = person
        .id
        .ok_or(DbError::Conflict("person id must be assigned".to_string()))?;
    let mut tx = pool.begin().await?;

    let result = sqlx::query!(
        "UPDATE persons SET first_names = $2, last_names = $3, orcid = $4, gnd = $5, ror = $6 WHERE id = $1",
        id,
        person.first_names,
        person.last_names,
        person.orcid.as_ref().map(|i| &i.value),
        person.gnd.as_ref().map(|i| &i.value),
        person.ror.as_ref().map(|i| &i.value),
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("person"));
    }

    replace_bios(&mut tx, id, person.bios.as_deref().unwrap_or(&[])).await?;

    tx.commit().await?;
    Ok(())
}

/// Deletes a person by id. Fails with [`DbError::NotFound`] if no row with that id exists.
///
/// Links from projects/sections are removed by the foreign-key cascade. Callers that want to
/// keep credits as free text first run [`convert_links_to_names`] in the same transaction.
pub async fn delete<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!("DELETE FROM persons WHERE id = $1", id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("person"));
    }
    Ok(())
}

/// Returns the scope of a person. Fails with [`DbError::NotFound`] if it doesn't exist.
pub async fn scope_of<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<PersonScope, DbError> {
    let row = sqlx::query!(
        "SELECT owner_team_id, project_id FROM persons WHERE id = $1",
        id
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("person"))?;
    match (row.owner_team_id, row.project_id) {
        (Some(team_id), _) => Ok(PersonScope::Team(team_id)),
        (None, Some(project_id)) => Ok(PersonScope::Project(project_id)),
        (None, None) => Err(DbError::Conflict("person has no scope".to_string())),
    }
}

/// Case-insensitive substring search across name/orcid/gnd/ror within `filter`. Biographies
/// are not fetched for search results (not needed by the person-picker UI these results feed).
pub async fn search(
    pool: &PgPool,
    query: &str,
    limit: i64,
    filter: ScopeFilter,
) -> Result<Vec<Person>, DbError> {
    let pattern = format!("%{}%", query);
    let (scope_condition, scope_id) = match filter {
        ScopeFilter::Team(team_id) => ("owner_team_id = $3", team_id),
        ScopeFilter::Project(project_id) => (
            "(project_id = $3 OR owner_team_id = (SELECT owner_team_id FROM projects WHERE id = $3))",
            project_id,
        ),
    };
    // Safe: `scope_condition` is one of the fixed strings above, never caller-supplied data.
    let sql = format!(
        "SELECT {PERSON_COLUMNS} FROM persons
         WHERE {scope_condition}
           AND (first_names ILIKE $1 OR last_names ILIKE $1
                OR orcid ILIKE $1 OR gnd ILIKE $1 OR ror ILIKE $1)
         ORDER BY last_names LIMIT $2"
    );
    let people: Vec<Person> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pattern)
        .bind(limit)
        .bind(scope_id)
        .fetch_all(pool)
        .await?;
    Ok(people)
}

/// Returns every person private to `project_id`, ordered by last name.
pub async fn list_for_project(pool: &PgPool, project_id: Uuid) -> Result<Vec<Person>, DbError> {
    let people: Vec<Person> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PERSON_COLUMNS} FROM persons WHERE project_id = $1 ORDER BY last_names"
    )))
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(people)
}

/// Sort order for [`list_for_team_paginated`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PersonOrderBy {
    FirstnameAscending,
    FirstnameDescending,
    LastnameAscending,
    LastnameDescending,
}

impl PersonOrderBy {
    /// Fixed SQL fragment for this variant (not user input, so safe to splice into the query).
    fn as_sql(self) -> &'static str {
        match self {
            PersonOrderBy::FirstnameAscending => "first_names ASC",
            PersonOrderBy::FirstnameDescending => "first_names DESC",
            PersonOrderBy::LastnameAscending => "last_names ASC",
            PersonOrderBy::LastnameDescending => "last_names DESC",
        }
    }
}

/// Returns one page of a team's person library, sorted and limited by the database
/// (`ORDER BY` / `LIMIT` / `OFFSET`) rather than in application code.
pub async fn list_for_team_paginated(
    pool: &PgPool,
    team_id: Uuid,
    order_by: PersonOrderBy,
    offset: i64,
    limit: i64,
) -> Result<Vec<Person>, DbError> {
    let query = format!(
        "SELECT {PERSON_COLUMNS} FROM persons WHERE owner_team_id = $1 ORDER BY {} LIMIT $2 OFFSET $3",
        order_by.as_sql()
    );
    // Safe: `order_by.as_sql()` only ever yields one of the fixed strings in `PersonOrderBy::as_sql`,
    // never caller-supplied data.
    let people: Vec<Person> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(team_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok(people)
}

/// Number of persons in a team's library.
pub async fn count_for_team(pool: &PgPool, team_id: Uuid) -> Result<i64, DbError> {
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM persons WHERE owner_team_id = $1",
        team_id
    )
    .fetch_one(pool)
    .await?;
    Ok(count.unwrap_or(0))
}

/// Of `person_ids`, returns those that may be linked as author/editor of `project_id`: they
/// are private to that very project, or part of the library of the team owning the project.
/// Unknown ids are simply absent from the result.
pub async fn linkable_ids<'e>(
    exec: impl PgExecutor<'e>,
    person_ids: &[Uuid],
    project_id: Uuid,
) -> Result<HashSet<Uuid>, DbError> {
    let ids = sqlx::query_scalar!(
        r#"SELECT p.id FROM persons p
           JOIN projects pr ON pr.id = $2
           WHERE p.id = ANY($1)
             AND (p.project_id = pr.id OR p.owner_team_id = pr.owner_team_id)"#,
        person_ids,
        project_id
    )
    .fetch_all(exec)
    .await?;
    Ok(ids.into_iter().collect())
}

/// Moves a project-private person into the library of the team owning that project. The id
/// stays the same, so existing links remain valid. Fails with [`DbError::NotFound`] for an
/// unknown person and [`DbError::Conflict`] if the person is already a team person.
pub async fn promote_to_team(pool: &PgPool, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE persons SET owner_team_id = pr.owner_team_id, project_id = NULL
         FROM projects pr
         WHERE persons.id = $1 AND persons.project_id = pr.id",
        id
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return if exists(pool, id).await? {
            Err(DbError::Conflict(
                "person already belongs to the team library".to_string(),
            ))
        } else {
            Err(DbError::NotFound("person"))
        };
    }
    Ok(())
}

/// Lists every project (and, if applicable, section) that links to the person as author/editor.
pub async fn usage<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<Vec<PersonUsage>, DbError> {
    let rows = sqlx::query!(
        r#"SELECT pr.id AS "project_id!", pr.title AS "project_title!", NULL::uuid AS section_id,
                  NULL::text AS section_title, pp.role AS "role!"
           FROM persons_projects pp
           JOIN projects pr ON pr.id = pp.project_id
           WHERE pp.person_id = $1
           UNION ALL
           SELECT pr.id, pr.title, s.id, s.title, ps.role
           FROM persons_sections ps
           JOIN sections s ON s.id = ps.section_id
           JOIN projects pr ON pr.id = s.project_id
           WHERE ps.person_id = $1
           ORDER BY 2, 4"#,
        id
    )
    .fetch_all(exec)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PersonUsage {
            project_id: r.project_id,
            project_title: r.project_title,
            section_id: r.section_id,
            section_title: r.section_title,
            role: r.role,
        })
        .collect())
}

/// Replaces every link to the person (project and section authors/editors) with a free-text
/// name, so that deleting the person afterwards keeps the credit visible. Free-text names are
/// project-local, which matches where the links lived anyway.
///
/// Returns the number of links that were converted.
pub async fn convert_links_to_names(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<u64, DbError> {
    let project_links = sqlx::query!(
        "UPDATE persons_projects
         SET name = (SELECT concat_ws(' ', first_names, last_names) FROM persons WHERE id = $1),
             person_id = NULL
         WHERE person_id = $1",
        id
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let section_links = sqlx::query!(
        "UPDATE persons_sections
         SET name = (SELECT concat_ws(' ', first_names, last_names) FROM persons WHERE id = $1),
             person_id = NULL
         WHERE person_id = $1",
        id
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(project_links + section_links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;
    use vb_exchange::projects::{Identifier, IdentifierType};

    fn sample_person(last_names: &str, scope: PersonScope) -> Person {
        Person {
            id: Some(Uuid::new_v4()),
            first_names: Some("First".to_string()),
            last_names: last_names.to_string(),
            orcid: Some(Identifier::new(
                IdentifierType::ORCID,
                "0000-0001".to_string(),
                None,
            )),
            gnd: None,
            ror: None,
            bios: Some(vec![Biography {
                content: "Bio text".to_string(),
                lang: None,
            }]),
            scope: Some(scope),
        }
    }

    async fn new_team(pool: &PgPool, name: &str) -> Uuid {
        sqlx::query_scalar!("INSERT INTO teams (name) VALUES ($1) RETURNING id", name)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn new_project(pool: &PgPool, team_id: Uuid, title: &str) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ($1, $2) RETURNING id",
            title,
            team_id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn link_to_project(pool: &PgPool, person_id: Uuid, project_id: Uuid) {
        sqlx::query!(
            "INSERT INTO persons_projects (person_id, project_id, role, position) VALUES ($1, $2, 'author', 0)",
            person_id,
            project_id
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn insert_and_get_round_trip(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let person = sample_person("Doe", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();

        let fetched = get(&pool, person.id.unwrap()).await.unwrap();
        assert_eq!(fetched.last_names, "Doe");
        assert_eq!(fetched.scope, Some(PersonScope::Team(team_id)));
        assert_eq!(
            fetched.orcid.as_ref().map(|i| i.value.clone()),
            Some("0000-0001".to_string())
        );
        assert_eq!(fetched.bios.unwrap().len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn insert_requires_a_scope(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let mut person = sample_person("Doe", PersonScope::Team(team_id));
        person.scope = None;
        assert!(matches!(
            insert(&pool, &person).await,
            Err(DbError::Conflict(_))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn scope_check_rejects_both_or_neither(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let project_id = new_project(&pool, team_id, "P").await;

        let neither = sqlx::query!("INSERT INTO persons (last_names) VALUES ('Nobody')")
            .execute(&pool)
            .await;
        assert!(neither.is_err());

        let both = sqlx::query!(
            "INSERT INTO persons (last_names, owner_team_id, project_id) VALUES ('Both', $1, $2)",
            team_id,
            project_id
        )
        .execute(&pool)
        .await;
        assert!(both.is_err());
        Ok(())
    }

    #[sqlx::test]
    async fn update_replaces_biographies_without_duplicates(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let mut person = sample_person("Smith", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();

        person.bios = Some(vec![Biography {
            content: "Updated bio".to_string(),
            lang: None,
        }]);
        update(&pool, &person).await.unwrap();

        let fetched = get(&pool, person.id.unwrap()).await.unwrap();
        let bios = fetched.bios.unwrap();
        assert_eq!(bios.len(), 1);
        assert_eq!(bios[0].content, "Updated bio");
        Ok(())
    }

    #[sqlx::test]
    async fn update_ignores_scope_in_body(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let other_team = new_team(&pool, "Other").await;
        let mut person = sample_person("Smith", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();

        person.scope = Some(PersonScope::Team(other_team));
        update(&pool, &person).await.unwrap();

        assert_eq!(
            scope_of(&pool, person.id.unwrap()).await.unwrap(),
            PersonScope::Team(team_id)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn delete_cascades_person_links(pool: PgPool) -> sqlx::Result<()> {
        let team_id = users::ensure_default_team(&pool).await.unwrap();
        let person = sample_person("Jones", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();
        let person_id = person.id.unwrap();

        let project_id = new_project(&pool, team_id, "Test").await;
        link_to_project(&pool, person_id, project_id).await;

        delete(&pool, person_id).await.unwrap();

        let remaining = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM persons_projects WHERE person_id = $1",
            person_id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(remaining.unwrap_or(-1), 0);
        Ok(())
    }

    #[sqlx::test]
    async fn search_matches_each_indexed_column(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "T").await;
        let filter = ScopeFilter::Team(team_id);
        insert(
            &pool,
            &sample_person("Zimmermann", PersonScope::Team(team_id)),
        )
        .await
        .unwrap();
        let by_last_name = search(&pool, "Zimmer", 10, filter).await.unwrap();
        assert_eq!(by_last_name.len(), 1);

        let by_orcid = search(&pool, "0000-0001", 10, filter).await.unwrap();
        assert_eq!(by_orcid.len(), 1);

        let no_match = search(&pool, "nonexistent", 10, filter).await.unwrap();
        assert!(no_match.is_empty());
        Ok(())
    }

    #[sqlx::test]
    async fn search_and_list_are_isolated_per_scope(pool: PgPool) -> sqlx::Result<()> {
        let team_a = new_team(&pool, "A").await;
        let team_b = new_team(&pool, "B").await;
        let project_a1 = new_project(&pool, team_a, "A1").await;
        let project_a2 = new_project(&pool, team_a, "A2").await;
        let project_b = new_project(&pool, team_b, "B1").await;

        insert(&pool, &sample_person("TeamA", PersonScope::Team(team_a)))
            .await
            .unwrap();
        insert(&pool, &sample_person("TeamB", PersonScope::Team(team_b)))
            .await
            .unwrap();
        insert(
            &pool,
            &sample_person("OnlyA1", PersonScope::Project(project_a1)),
        )
        .await
        .unwrap();
        insert(
            &pool,
            &sample_person("OnlyB", PersonScope::Project(project_b)),
        )
        .await
        .unwrap();

        let names = |people: Vec<Person>| -> Vec<String> {
            people.into_iter().map(|p| p.last_names).collect()
        };

        // team library: library persons only, never project persons or other teams
        assert_eq!(
            names(
                search(&pool, "", 10, ScopeFilter::Team(team_a))
                    .await
                    .unwrap()
            ),
            ["TeamA"]
        );
        // project view: its own persons + its team's library
        assert_eq!(
            names(
                search(&pool, "", 10, ScopeFilter::Project(project_a1))
                    .await
                    .unwrap()
            ),
            ["OnlyA1", "TeamA"]
        );
        // a sibling project in the same team doesn't see A1's private person
        assert_eq!(
            names(
                search(&pool, "", 10, ScopeFilter::Project(project_a2))
                    .await
                    .unwrap()
            ),
            ["TeamA"]
        );

        assert_eq!(count_for_team(&pool, team_a).await.unwrap(), 1);
        let page = list_for_team_paginated(&pool, team_b, PersonOrderBy::LastnameAscending, 0, 10)
            .await
            .unwrap();
        assert_eq!(names(page), ["TeamB"]);
        assert_eq!(
            names(list_for_project(&pool, project_a1).await.unwrap()),
            ["OnlyA1"]
        );
        Ok(())
    }

    #[sqlx::test]
    async fn linkable_ids_follow_scope(pool: PgPool) -> sqlx::Result<()> {
        let team_a = new_team(&pool, "A").await;
        let team_b = new_team(&pool, "B").await;
        let project_a1 = new_project(&pool, team_a, "A1").await;
        let project_a2 = new_project(&pool, team_a, "A2").await;
        let project_b = new_project(&pool, team_b, "B1").await;

        let library = sample_person("Library", PersonScope::Team(team_a));
        let private = sample_person("Private", PersonScope::Project(project_a1));
        insert(&pool, &library).await.unwrap();
        insert(&pool, &private).await.unwrap();
        let (library, private) = (library.id.unwrap(), private.id.unwrap());

        let unknown = Uuid::new_v4();
        let all = [library, private, unknown];
        let linkable = |project| {
            let pool = pool.clone();
            async move { linkable_ids(&pool, &all, project).await.unwrap() }
        };
        assert_eq!(
            linkable(project_a1).await,
            HashSet::from([library, private])
        );
        assert_eq!(linkable(project_a2).await, HashSet::from([library]));
        assert!(linkable(project_b).await.is_empty());
        assert!(
            linkable_ids(&pool, &[], project_a1)
                .await
                .unwrap()
                .is_empty()
        );
        Ok(())
    }

    #[sqlx::test]
    async fn promote_moves_person_to_team_and_keeps_links(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "A").await;
        let project_id = new_project(&pool, team_id, "A1").await;
        let person = sample_person("Private", PersonScope::Project(project_id));
        insert(&pool, &person).await.unwrap();
        let id = person.id.unwrap();
        link_to_project(&pool, id, project_id).await;

        promote_to_team(&pool, id).await.unwrap();
        assert_eq!(
            scope_of(&pool, id).await.unwrap(),
            PersonScope::Team(team_id)
        );
        assert_eq!(usage(&pool, id).await.unwrap().len(), 1);

        assert!(matches!(
            promote_to_team(&pool, id).await,
            Err(DbError::Conflict(_))
        ));
        assert!(matches!(
            promote_to_team(&pool, Uuid::new_v4()).await,
            Err(DbError::NotFound("person"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn usage_lists_project_and_section_links(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "A").await;
        let project_id = new_project(&pool, team_id, "My book").await;
        let person = sample_person("Author", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();
        let id = person.id.unwrap();
        assert!(usage(&pool, id).await.unwrap().is_empty());

        link_to_project(&pool, id, project_id).await;
        let section_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO sections (project_id, position, title) VALUES ($1, 0, 'Chapter 1') RETURNING id",
            project_id
        )
        .fetch_one(&pool)
        .await?;
        sqlx::query!(
            "INSERT INTO persons_sections (person_id, section_id, role, position) VALUES ($1, $2, 'editor', 0)",
            id,
            section_id
        )
        .execute(&pool)
        .await?;

        let usages = usage(&pool, id).await.unwrap();
        assert_eq!(usages.len(), 2);
        assert!(
            usages
                .iter()
                .all(|u| u.project_id == project_id && u.project_title == "My book")
        );
        assert!(
            usages
                .iter()
                .any(|u| u.section_id.is_none() && u.role == "author")
        );
        assert!(usages.iter().any(|u| u.section_id == Some(section_id)
            && u.section_title.as_deref() == Some("Chapter 1")
            && u.role == "editor"));
        Ok(())
    }

    #[sqlx::test]
    async fn convert_links_to_names_keeps_credit_after_delete(pool: PgPool) -> sqlx::Result<()> {
        let team_id = new_team(&pool, "A").await;
        let project_id = new_project(&pool, team_id, "My book").await;
        let person = sample_person("Author", PersonScope::Team(team_id));
        insert(&pool, &person).await.unwrap();
        let id = person.id.unwrap();
        link_to_project(&pool, id, project_id).await;

        let mut tx = pool.begin().await?;
        assert_eq!(convert_links_to_names(&mut tx, id).await.unwrap(), 1);
        delete(&mut *tx, id).await.unwrap();
        tx.commit().await?;

        let name: Option<String> = sqlx::query_scalar!(
            "SELECT name FROM persons_projects WHERE project_id = $1",
            project_id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(name.as_deref(), Some("First Author"));
        assert!(!exists(&pool, id).await.unwrap());
        Ok(())
    }
}
