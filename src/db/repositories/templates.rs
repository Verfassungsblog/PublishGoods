//! `project_templates` / `export_formats`.
//!
//! [`ProjectTemplateV2`] and [`ExportFormat`]/[`ExportStep`] can't be targeted directly by
//! `query_as!`/`FromRow`: `ProjectTemplateV2.export_formats` and `ExportFormat.export_steps`
//! require multi-row/jsonb assembly, and `ExportFormat`/`ExportStep` are external-crate types
//! (orphan rule blocks a `FromRow` impl for them here). We fetch raw columns with the
//! anonymous-struct `query!` macro and assemble the domain structs by hand.

use super::DbError;
use sqlx::PgConnection;
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use sqlx::types::Json;
use std::collections::HashMap;
use uuid::Uuid;
use vb_exchange::export_formats::{ExportFormat, ExportStep};

/// Loads the export format rows of every template in `template_ids` in one query, keyed by
/// template id and then by slug. Templates without export formats are absent from the map.
async fn fetch_export_formats(
    pool: &PgPool,
    template_ids: &[Uuid],
) -> Result<HashMap<Uuid, HashMap<String, ExportFormat>>, DbError> {
    let rows = sqlx::query!(
        r#"SELECT project_template_id, slug, name, preview_pdf_path, output_files,
                  export_steps as "export_steps: Json<Vec<ExportStep>>"
           FROM export_formats WHERE project_template_id = ANY($1)"#,
        template_ids
    )
    .fetch_all(pool)
    .await?;

    let mut formats: HashMap<Uuid, HashMap<String, ExportFormat>> = HashMap::new();
    for row in rows {
        formats.entry(row.project_template_id).or_default().insert(
            row.slug.clone(),
            ExportFormat {
                slug: row.slug,
                name: row.name,
                export_steps: row
                    .export_steps
                    .map(|Json(steps)| steps)
                    .unwrap_or_default(),
                output_files: row.output_files.unwrap_or_default(),
                preview_pdf_path: row.preview_pdf_path,
            },
        );
    }
    Ok(formats)
}

/// Inserts a single `export_formats` row for `format` under `template_id`.
async fn insert_export_format_row<'e>(
    exec: impl PgExecutor<'e>,
    template_id: Uuid,
    format: &ExportFormat,
) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT INTO export_formats (project_template_id, slug, name, preview_pdf_path, output_files, export_steps)
         VALUES ($1, $2, $3, $4, $5, $6)",
        template_id,
        format.slug,
        format.name,
        format.preview_pdf_path,
        &format.output_files,
        Json(&format.export_steps) as Json<&Vec<ExportStep>>,
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// A project template, assembled from `project_templates` plus its `export_formats` rows.
/// Every template is owned by exactly one team (`owner_team_id`); only that team's members may
/// see or edit it, and only that team's projects may use it.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Template {
    pub id: Uuid,
    pub version: Uuid,
    pub name: String,
    pub description: String,
    pub owner_team_id: Uuid,
    pub export_formats: HashMap<String, ExportFormat>,
}

/// Fetches a template by id, including its export formats.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Template, DbError> {
    let row = sqlx::query!(
        "SELECT id, version, name, description, owner_team_id FROM project_templates WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound("template"))?;

    let export_formats = fetch_export_formats(pool, &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();

    Ok(Template {
        id: row.id,
        version: row.version,
        name: row.name,
        description: row.description.unwrap_or_default(),
        owner_team_id: row.owner_team_id,
        export_formats,
    })
}

/// Ids of all templates.
pub async fn list_ids(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    Ok(sqlx::query_scalar("SELECT id FROM project_templates")
        .fetch_all(pool)
        .await?)
}

/// Ids of the templates owned by the teams `team_ids`, for a transaction that deletes those
/// teams (and with them, by cascade, the templates). The team rows are locked first: creating a
/// template takes a share lock on its team through the foreign key, so one that is created
/// concurrently either is listed here or has to wait and then fails once the teams are gone.
/// Without that, a template could be deleted without ever being reported, and its files would
/// stay on disk.
pub async fn list_ids_for_teams_being_deleted(
    conn: &mut PgConnection,
    team_ids: &[Uuid],
) -> Result<Vec<Uuid>, DbError> {
    // ordered, so transactions deleting overlapping sets of teams can't deadlock
    sqlx::query("SELECT id FROM teams WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(team_ids)
        .execute(&mut *conn)
        .await?;
    Ok(
        sqlx::query_scalar("SELECT id FROM project_templates WHERE owner_team_id = ANY($1)")
            .bind(team_ids)
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// Checks whether a template with the given id exists.
pub async fn exists<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<bool, DbError> {
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_templates WHERE id = $1)",
        id
    )
    .fetch_one(exec)
    .await?;
    Ok(exists.unwrap_or(false))
}

/// A template without its export formats: all a template picker needs.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct TemplateSummary {
    pub id: Uuid,
    pub version: Uuid,
    pub name: String,
    pub description: String,
    pub owner_team_id: Uuid,
}

/// Populates the export formats of `summaries` (one query, independent of their number).
async fn with_export_formats(
    pool: &PgPool,
    summaries: Vec<TemplateSummary>,
) -> Result<Vec<Template>, DbError> {
    let ids: Vec<Uuid> = summaries.iter().map(|summary| summary.id).collect();
    let mut export_formats = fetch_export_formats(pool, &ids).await?;
    Ok(summaries
        .into_iter()
        .map(|summary| Template {
            export_formats: export_formats.remove(&summary.id).unwrap_or_default(),
            id: summary.id,
            version: summary.version,
            name: summary.name,
            description: summary.description,
            owner_team_id: summary.owner_team_id,
        })
        .collect())
}

/// Returns the templates of one team, ordered by name, each with its export formats populated.
pub async fn list_for_team(pool: &PgPool, team_id: Uuid) -> Result<Vec<Template>, DbError> {
    let summaries = sqlx::query_as!(
        TemplateSummary,
        r#"SELECT id, version, name, COALESCE(description, '') AS "description!", owner_team_id
           FROM project_templates WHERE owner_team_id = $1 ORDER BY name"#,
        team_id
    )
    .fetch_all(pool)
    .await?;
    with_export_formats(pool, summaries).await
}

/// Returns the templates of every team `user_id` is a member of, ordered by name, without
/// their export formats.
pub async fn list_summaries_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<TemplateSummary>, DbError> {
    Ok(sqlx::query_as!(
        TemplateSummary,
        r#"SELECT id, version, name, COALESCE(description, '') AS "description!", owner_team_id
           FROM project_templates
           WHERE owner_team_id IN (SELECT team_id FROM users_teams WHERE user_id = $1)
           ORDER BY name"#,
        user_id
    )
    .fetch_all(pool)
    .await?)
}

/// Returns the templates of every team `user_id` is a member of, ordered by name, each with its
/// export formats populated.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Template>, DbError> {
    let summaries = list_summaries_for_user(pool, user_id).await?;
    with_export_formats(pool, summaries).await
}

/// Returns the templates `project_id` may use (those of the team owning it), ordered by name,
/// each with its export formats populated.
pub async fn list_for_project(pool: &PgPool, project_id: Uuid) -> Result<Vec<Template>, DbError> {
    let summaries = sqlx::query_as!(
        TemplateSummary,
        r#"SELECT id, version, name, COALESCE(description, '') AS "description!", owner_team_id
           FROM project_templates
           WHERE owner_team_id = (SELECT owner_team_id FROM projects WHERE id = $1)
           ORDER BY name"#,
        project_id
    )
    .fetch_all(pool)
    .await?;
    with_export_formats(pool, summaries).await
}

/// Creates a new, empty (no export formats) template owned by `team_id`, returning its new id.
pub async fn insert(
    pool: &PgPool,
    team_id: Uuid,
    name: &str,
    description: &str,
) -> Result<Uuid, DbError> {
    let id = sqlx::query_scalar!(
        "INSERT INTO project_templates (name, description, owner_team_id) VALUES ($1, $2, $3) RETURNING id",
        name,
        description,
        team_id
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fully replaces a template's name/description and its entire set of export formats
/// (delete-then-reinsert), bumping `version`. The owning team never changes. Mirrors the old code's whole-struct overwrite
/// semantics for `PUT /api/templates/<id>`.
pub async fn replace(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    description: &str,
    export_formats: &HashMap<String, ExportFormat>,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    let result = sqlx::query!(
        "UPDATE project_templates SET name = $2, description = $3, version = gen_random_uuid() WHERE id = $1",
        id,
        name,
        description
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("template"));
    }

    sqlx::query!(
        "DELETE FROM export_formats WHERE project_template_id = $1",
        id
    )
    .execute(&mut *tx)
    .await?;
    for format in export_formats.values() {
        insert_export_format_row(&mut *tx, id, format).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Bumps a template's `version` without touching any other field. Fails with
/// [`DbError::NotFound`] if no template with that id exists.
pub async fn touch_version<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE project_templates SET version = gen_random_uuid() WHERE id = $1",
        id
    )
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("template"));
    }
    Ok(())
}

/// Checks whether an export format with the given slug exists for a template.
pub async fn export_format_exists(
    pool: &PgPool,
    template_id: Uuid,
    slug: &str,
) -> Result<bool, DbError> {
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM export_formats WHERE project_template_id = $1 AND slug = $2)",
        template_id,
        slug
    )
    .fetch_one(pool)
    .await?;
    Ok(exists.unwrap_or(false))
}

/// Adds a new export format to a template. Fails with [`DbError::NotFound`] if the
/// template doesn't exist, or [`DbError::Conflict`] if its slug is already in use.
pub async fn insert_export_format(
    pool: &PgPool,
    template_id: Uuid,
    format: &ExportFormat,
) -> Result<(), DbError> {
    if !exists(pool, template_id).await? {
        return Err(DbError::NotFound("template"));
    }
    if export_format_exists(pool, template_id, &format.slug).await? {
        return Err(DbError::Conflict(
            "An export format with this slug already exists.".to_string(),
        ));
    }
    insert_export_format_row(pool, template_id, format).await
}

/// Fetches a single export format by slug.
pub async fn get_export_format(
    pool: &PgPool,
    template_id: Uuid,
    slug: &str,
) -> Result<ExportFormat, DbError> {
    let row = sqlx::query!(
        r#"SELECT slug, name, preview_pdf_path, output_files,
                  export_steps as "export_steps: Json<Vec<ExportStep>>"
           FROM export_formats WHERE project_template_id = $1 AND slug = $2"#,
        template_id,
        slug
    )
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound("export_format"))?;

    Ok(ExportFormat {
        slug: row.slug,
        name: row.name,
        export_steps: row
            .export_steps
            .map(|Json(steps)| steps)
            .unwrap_or_default(),
        output_files: row.output_files.unwrap_or_default(),
        preview_pdf_path: row.preview_pdf_path,
    })
}

/// Updates name/slug/preview_pdf_path for an export format, keeping its export_steps intact.
/// Returns the export format's (possibly new) slug.
pub async fn update_export_format_metadata(
    pool: &PgPool,
    template_id: Uuid,
    old_slug: &str,
    new_slug: &str,
    name: &str,
    preview_pdf_path: Option<&str>,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE export_formats SET slug = $3, name = $4, preview_pdf_path = $5
         WHERE project_template_id = $1 AND slug = $2",
        template_id,
        old_slug,
        new_slug,
        name,
        preview_pdf_path
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("export_format"));
    }
    Ok(())
}

/// Deletes an export format by slug. Fails with [`DbError::NotFound`] if it doesn't exist.
pub async fn delete_export_format(
    pool: &PgPool,
    template_id: Uuid,
    slug: &str,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "DELETE FROM export_formats WHERE project_template_id = $1 AND slug = $2",
        template_id,
        slug
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("export_format"));
    }
    Ok(())
}

/// Fetches just the `export_steps` of one export format.
pub async fn get_export_steps(
    pool: &PgPool,
    template_id: Uuid,
    slug: &str,
) -> Result<Vec<ExportStep>, DbError> {
    Ok(get_export_format(pool, template_id, slug)
        .await?
        .export_steps)
}

/// Read-modify-write for the `export_steps` jsonb column: callers fetch via
/// [`get_export_steps`], mutate the `Vec` in Rust exactly like the old in-memory code did,
/// then write the whole array back here.
pub async fn update_export_steps(
    pool: &PgPool,
    template_id: Uuid,
    slug: &str,
    steps: &[ExportStep],
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE export_formats SET export_steps = $3 WHERE project_template_id = $1 AND slug = $2",
        template_id,
        slug,
        Json(steps) as Json<&[ExportStep]>,
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("export_format"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_exchange::export_formats::{ExportStepData, RawExportStep};

    async fn team(pool: &PgPool) -> Uuid {
        sqlx::query_scalar!("INSERT INTO teams (name) VALUES ('T') RETURNING id")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn sample_format(slug: &str) -> ExportFormat {
        ExportFormat {
            slug: slug.to_string(),
            name: "PDF".to_string(),
            export_steps: vec![ExportStep {
                id: Some(Uuid::new_v4()),
                name: "step1".to_string(),
                data: ExportStepData::Raw(RawExportStep {
                    entry_point: "main.hbs".to_string(),
                    output_file: "main.html".to_string(),
                }),
                files_to_keep: vec![],
            }],
            output_files: vec!["main.html".to_string()],
            preview_pdf_path: None,
        }
    }

    #[sqlx::test]
    async fn insert_and_get_with_export_formats(pool: PgPool) -> sqlx::Result<()> {
        let id = insert(&pool, team(&pool).await, "My Template", "A template")
            .await
            .unwrap();
        insert_export_format(&pool, id, &sample_format("pdf"))
            .await
            .unwrap();

        let template = get(&pool, id).await.unwrap();
        assert_eq!(template.name, "My Template");
        assert_eq!(template.export_formats.len(), 1);
        assert_eq!(template.export_formats["pdf"].export_steps.len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn touch_version_changes_version(pool: PgPool) -> sqlx::Result<()> {
        let id = insert(&pool, team(&pool).await, "T", "D").await.unwrap();
        let before = get(&pool, id).await.unwrap().version;
        touch_version(&pool, id).await.unwrap();
        let after = get(&pool, id).await.unwrap().version;
        assert_ne!(before, after);
        Ok(())
    }

    #[sqlx::test]
    async fn export_steps_read_modify_write_only_touches_target(pool: PgPool) -> sqlx::Result<()> {
        let id = insert(&pool, team(&pool).await, "T", "D").await.unwrap();
        insert_export_format(&pool, id, &sample_format("pdf"))
            .await
            .unwrap();
        insert_export_format(&pool, id, &sample_format("epub"))
            .await
            .unwrap();

        let mut steps = get_export_steps(&pool, id, "pdf").await.unwrap();
        steps[0].name = "renamed".to_string();
        update_export_steps(&pool, id, "pdf", &steps).await.unwrap();

        let pdf_steps = get_export_steps(&pool, id, "pdf").await.unwrap();
        assert_eq!(pdf_steps[0].name, "renamed");
        let epub_steps = get_export_steps(&pool, id, "epub").await.unwrap();
        assert_eq!(epub_steps[0].name, "step1");
        Ok(())
    }

    #[sqlx::test]
    async fn insert_export_format_rejects_duplicate_slug(pool: PgPool) -> sqlx::Result<()> {
        let id = insert(&pool, team(&pool).await, "T", "D").await.unwrap();
        insert_export_format(&pool, id, &sample_format("pdf"))
            .await
            .unwrap();
        let result = insert_export_format(&pool, id, &sample_format("pdf")).await;
        assert!(matches!(result, Err(DbError::Conflict(_))));
        Ok(())
    }

    #[sqlx::test]
    async fn listings_are_scoped_to_the_owning_team(pool: PgPool) -> sqlx::Result<()> {
        let (a, b) = (team(&pool).await, team(&pool).await);
        let ta = insert(&pool, a, "A", "").await.unwrap();
        let tb = insert(&pool, b, "B", "").await.unwrap();

        let ids = |ts: Vec<Template>| ts.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(list_for_team(&pool, a).await.unwrap()), vec![ta]);
        assert_eq!(ids(list_for_team(&pool, b).await.unwrap()), vec![tb]);
        let project = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO projects (id, title, owner_team_id) VALUES ($1, 'p', $2)",
            project,
            a
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            ids(list_for_project(&pool, project).await.unwrap()),
            vec![ta]
        );

        let user = sqlx::query_scalar!(
            "INSERT INTO users (email, name, password_hash) VALUES ('u@example.com', 'u', 'h') RETURNING id"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, 'member')",
            user,
            a
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(ids(list_for_user(&pool, user).await.unwrap()), vec![ta]);
        Ok(())
    }
}
