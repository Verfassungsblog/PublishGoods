//! `persons` / `biographies`.
//!
//! [`vb_exchange::projects::PersonV2`]/[`Biography`] are defined in an external crate
//! (`vb-exchange`), which implements `sqlx::FromRow` for both directly (gated behind its
//! `sqlx` feature) — so reads here use `sqlx::query_as` straight into the domain structs,
//! no named intermediate row type. `PersonV2::bios` isn't a plain column (biographies live in
//! their own table), so `FromRow` always leaves it `None`; callers that need it fetch
//! separately via [`fetch_bios`] and fill the field in afterward. Because `query_as` (not the
//! compile-time-checked `query_as!` macro) is what consumes a `FromRow` impl, these read
//! queries are runtime-checked rather than macro-checked.

use super::DbError;
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;
use vb_exchange::projects::{Biography, PersonV2};

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

/// Fetches a person by id, including their biographies.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<PersonV2, DbError> {
    let mut person: PersonV2 = sqlx::query_as(
        "SELECT id, first_names, last_names, orcid, gnd, ror FROM persons WHERE id = $1",
    )
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
/// API-level id-assignment convention) plus its biographies, in one transaction.
pub async fn insert(pool: &PgPool, person: &PersonV2) -> Result<(), DbError> {
    let id = person
        .id
        .ok_or(DbError::Conflict("person id must be assigned".to_string()))?;
    let mut tx = pool.begin().await?;

    sqlx::query!(
        "INSERT INTO persons (id, first_names, last_names, orcid, gnd, ror) VALUES ($1, $2, $3, $4, $5, $6)",
        id,
        person.first_names,
        person.last_names,
        person.orcid.as_ref().map(|i| &i.value),
        person.gnd.as_ref().map(|i| &i.value),
        person.ror.as_ref().map(|i| &i.value),
    )
    .execute(&mut *tx)
    .await?;

    replace_bios(&mut tx, id, person.bios.as_deref().unwrap_or(&[])).await?;

    tx.commit().await?;
    Ok(())
}

/// Updates `person`'s core fields and replaces its biographies, in one transaction.
/// Fails with [`DbError::NotFound`] if no row with that id exists.
pub async fn update(pool: &PgPool, person: &PersonV2) -> Result<(), DbError> {
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
pub async fn delete<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!("DELETE FROM persons WHERE id = $1", id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("person"));
    }
    Ok(())
}

/// Case-insensitive substring search across name/orcid/gnd/ror. Biographies are not
/// fetched for search results (not needed by the person-picker UI these results feed).
pub async fn search(pool: &PgPool, query: &str, limit: i64) -> Result<Vec<PersonV2>, DbError> {
    let pattern = format!("%{}%", query);
    let people: Vec<PersonV2> = sqlx::query_as(
        "SELECT id, first_names, last_names, orcid, gnd, ror FROM persons
         WHERE first_names ILIKE $1 OR last_names ILIKE $1
            OR orcid ILIKE $1 OR gnd ILIKE $1 OR ror ILIKE $1
         ORDER BY last_names LIMIT $2",
    )
    .bind(pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(people)
}

/// Returns every person in the database, ordered by last name.
pub async fn list_all(pool: &PgPool) -> Result<Vec<PersonV2>, DbError> {
    search(pool, "", i64::MAX).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_exchange::projects::{Identifier, IdentifierType};

    fn sample_person(last_names: &str) -> PersonV2 {
        PersonV2 {
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
        }
    }

    #[sqlx::test]
    async fn insert_and_get_round_trip(pool: PgPool) -> sqlx::Result<()> {
        let person = sample_person("Doe");
        insert(&pool, &person).await.unwrap();

        let fetched = get(&pool, person.id.unwrap()).await.unwrap();
        assert_eq!(fetched.last_names, "Doe");
        assert_eq!(
            fetched.orcid.as_ref().map(|i| i.value.clone()),
            Some("0000-0001".to_string())
        );
        assert_eq!(fetched.bios.unwrap().len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn update_replaces_biographies_without_duplicates(pool: PgPool) -> sqlx::Result<()> {
        let mut person = sample_person("Smith");
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
    async fn delete_cascades_person_links(pool: PgPool) -> sqlx::Result<()> {
        let person = sample_person("Jones");
        insert(&pool, &person).await.unwrap();
        let person_id = person.id.unwrap();

        let team_id = super::super::users::ensure_default_team(&pool)
            .await
            .unwrap();
        let project_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ('Test', $1) RETURNING id",
            team_id
        )
        .fetch_one(&pool)
        .await?;
        sqlx::query!(
            "INSERT INTO persons_projects (person_id, project_id, role, position) VALUES ($1, $2, 'author', 0)",
            person_id,
            project_id
        )
        .execute(&pool)
        .await?;

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
        insert(&pool, &sample_person("Zimmermann")).await.unwrap();
        let by_last_name = search(&pool, "Zimmer", 10).await.unwrap();
        assert_eq!(by_last_name.len(), 1);

        let by_orcid = search(&pool, "0000-0001", 10).await.unwrap();
        assert_eq!(by_orcid.len(), 1);

        let no_match = search(&pool, "nonexistent", 10).await.unwrap();
        assert!(no_match.is_empty());
        Ok(())
    }
}
