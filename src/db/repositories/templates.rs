//! `project_templates` / `export_formats`.
//!
//! [`ProjectTemplateV2`] and [`ExportFormat`]/[`ExportStep`] can't be targeted directly by
//! `query_as!`/`FromRow`: `ProjectTemplateV2.export_formats` and `ExportFormat.export_steps`
//! require multi-row/jsonb assembly, and `ExportFormat`/`ExportStep` are external-crate types
//! (orphan rule blocks a `FromRow` impl for them here). We fetch raw columns with the
//! anonymous-struct `query!` macro and assemble the domain structs by hand.

use super::DbError;
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use sqlx::types::Json;
use std::collections::HashMap;
use uuid::Uuid;
use vb_exchange::export_formats::{ExportFormat, ExportStep};

/// Loads every export format row for a template, keyed by slug.
async fn fetch_export_formats(
    pool: &PgPool,
    template_id: Uuid,
) -> Result<HashMap<String, ExportFormat>, DbError> {
    let rows = sqlx::query!(
        r#"SELECT slug, name, preview_pdf_path, output_files,
                  export_steps as "export_steps: Json<Vec<ExportStep>>"
           FROM export_formats WHERE project_template_id = $1"#,
        template_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
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
            )
        })
        .collect())
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Template {
    pub id: Uuid,
    pub version: Uuid,
    pub name: String,
    pub description: String,
    pub export_formats: HashMap<String, ExportFormat>,
}

/// Fetches a template by id, including its export formats.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Template, DbError> {
    let row = sqlx::query!(
        "SELECT id, version, name, description FROM project_templates WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound("template"))?;

    let export_formats = fetch_export_formats(pool, id).await?;

    Ok(Template {
        id: row.id,
        version: row.version,
        name: row.name,
        description: row.description.unwrap_or_default(),
        export_formats,
    })
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

/// Returns every template, ordered by name, each with its export formats populated.
pub async fn list_all(pool: &PgPool) -> Result<Vec<Template>, DbError> {
    let rows =
        sqlx::query!("SELECT id, version, name, description FROM project_templates ORDER BY name")
            .fetch_all(pool)
            .await?;

    let mut templates = Vec::with_capacity(rows.len());
    for row in rows {
        let export_formats = fetch_export_formats(pool, row.id).await?;
        templates.push(Template {
            id: row.id,
            version: row.version,
            name: row.name,
            description: row.description.unwrap_or_default(),
            export_formats,
        });
    }
    Ok(templates)
}

/// Creates a new, empty (no export formats) template, returning its new id.
pub async fn insert(pool: &PgPool, name: &str, description: &str) -> Result<Uuid, DbError> {
    let id = sqlx::query_scalar!(
        "INSERT INTO project_templates (name, description) VALUES ($1, $2) RETURNING id",
        name,
        description
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fully replaces a template's name/description and its entire set of export formats
/// (delete-then-reinsert), bumping `version`. Mirrors the old code's whole-struct overwrite
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
        let id = insert(&pool, "My Template", "A template").await.unwrap();
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
        let id = insert(&pool, "T", "D").await.unwrap();
        let before = get(&pool, id).await.unwrap().version;
        touch_version(&pool, id).await.unwrap();
        let after = get(&pool, id).await.unwrap().version;
        assert_ne!(before, after);
        Ok(())
    }

    #[sqlx::test]
    async fn export_steps_read_modify_write_only_touches_target(pool: PgPool) -> sqlx::Result<()> {
        let id = insert(&pool, "T", "D").await.unwrap();
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
        let id = insert(&pool, "T", "D").await.unwrap();
        insert_export_format(&pool, id, &sample_format("pdf"))
            .await
            .unwrap();
        let result = insert_export_format(&pool, id, &sample_format("pdf")).await;
        assert!(matches!(result, Err(DbError::Conflict(_))));
        Ok(())
    }
}
