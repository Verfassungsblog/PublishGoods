pub mod api;
pub mod user_interface;

/// HTTP-level coverage of template ownership: templates are only visible, editable and usable
/// within the team that owns them.
#[cfg(test)]
mod integration_tests {
    use crate::db::repositories::{projects, teams, templates, users};
    use crate::projects::api::get::integration_tests::dummy_settings;
    use crate::session::session_storage::SessionStorage;
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;
    use rocket_dyn_templates::Template;
    use sqlx::PgPool;
    use uuid::Uuid;

    struct World {
        pool: PgPool,
        /// alice's team, owning `template` and `project`
        team: Uuid,
        /// bob's team, owning `foreign_template`; alice is not a member
        other_team: Uuid,
        template: Uuid,
        foreign_template: Uuid,
        project: Uuid,
        alice: Client,
    }

    async fn login(pool: &PgPool, email: &str) -> Client {
        let client = anonymous(pool).await;
        {
            let response = client
                .post("/login")
                .header(ContentType::Form)
                .body(format!(
                    "email={}&password=correct+horse",
                    email.replace('@', "%40")
                ))
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::SeeOther);
        }
        client
    }

    /// A client without a session.
    async fn anonymous(pool: &PgPool) -> Client {
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool.clone())
            .manage(dummy_settings())
            .manage(SessionStorage::new())
            .attach(Template::fairing())
            .mount(
                "/",
                routes![
                    crate::session::login::login_page,
                    crate::session::login::process_login_form,
                    super::user_interface::list_templates,
                    super::user_interface::get_template,
                    super::user_interface::create_template,
                    super::user_interface::form_create_template,
                    super::api::get_template,
                    super::api::update_template,
                    super::api::get_assets,
                    super::api::get_assets_for_export_format,
                    super::api::get_asset_file,
                    super::api::get_export_steps,
                    super::api::move_asset,
                    super::api::delete_assets,
                    super::api::update_export_format_metadata,
                    crate::projects::api::get_project_template,
                    crate::projects::api::set_project_template,
                    crate::projects::api::list_project_templates,
                    crate::projects::api::list_templates,
                    crate::projects::create::process_create_project,
                    crate::projects::api::patch::patch_project,
                ],
            );
        Client::tracked(rocket).await.unwrap()
    }

    async fn world(pool: PgPool) -> World {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let hash = Argon2::default()
            .hash_password(b"correct horse", &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string();
        let alice = users::insert(&pool, "alice@example.com", "Alice", &hash, default_team)
            .await
            .unwrap();
        let bob = users::insert(&pool, "bob@example.com", "Bob", &hash, default_team)
            .await
            .unwrap();
        let team = teams::create(&pool, "Lib", alice.id).await.unwrap().id;
        let other_team = teams::create(&pool, "Elsewhere", bob.id).await.unwrap().id;
        let template = templates::insert(&pool, team, "Own <b>Book</b>", "mine")
            .await
            .unwrap();
        let foreign_template = templates::insert(&pool, other_team, "Foreign", "theirs")
            .await
            .unwrap();
        let project = Uuid::new_v4();
        projects::insert(&pool, project, "Book", None, Some(template), None, team)
            .await
            .unwrap();

        World {
            alice: login(&pool, "alice@example.com").await,
            pool,
            team,
            other_team,
            template,
            foreign_template,
            project,
        }
    }

    async fn status(client: &Client, uri: String) -> Status {
        client.get(uri).dispatch().await.status()
    }

    #[sqlx::test]
    async fn pages_only_show_the_selected_own_team(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;

        let response = w
            .alice
            .get(format!("/templates?team_id={}", w.team))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let page = response.into_string().await.unwrap();
        // names are HTML-escaped
        assert!(page.contains("Own &lt;b&gt;Book&lt;/b&gt;"));
        assert!(!page.contains("Foreign"));

        let foreign_team = format!("/templates?team_id={}", w.other_team);
        assert_eq!(status(&w.alice, foreign_team).await, Status::NotFound);
        let create_for_foreign = format!("/templates/create?team_id={}", w.other_team);
        assert_eq!(status(&w.alice, create_for_foreign).await, Status::NotFound);
        assert_eq!(
            status(&w.alice, format!("/templates/create?team_id={}", w.team)).await,
            Status::Ok
        );
        assert_eq!(
            status(&w.alice, format!("/templates/{}", w.template)).await,
            Status::Ok
        );
        assert_eq!(
            status(&w.alice, format!("/templates/{}", w.foreign_template)).await,
            Status::NotFound
        );
        Ok(())
    }

    #[sqlx::test]
    async fn authorized_guard_requires_a_session_and_a_valid_id(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let anonymous = anonymous(&w.pool).await;
        for uri in [
            format!("/api/templates/{}", w.template),
            format!("/api/templates/{}/assets", w.template),
            format!("/templates/{}", w.template),
        ] {
            assert_eq!(
                status(&anonymous, uri.clone()).await,
                Status::Unauthorized,
                "{uri}"
            );
        }

        assert_eq!(
            status(&w.alice, "/api/templates/not-a-uuid".to_string()).await,
            Status::BadRequest
        );
        assert_eq!(
            status(&w.alice, format!("/api/templates/{}", Uuid::new_v4())).await,
            Status::NotFound
        );
        // file routes keep answering 404 for a malformed id
        assert_eq!(
            status(
                &w.alice,
                "/api/templates/not-a-uuid/assets/files/a.txt".to_string()
            )
            .await,
            Status::NotFound
        );
        Ok(())
    }

    #[sqlx::test]
    async fn export_format_routes_reject_traversal_slugs(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        // `.....///..` decodes from `.....%2F%2F%2F..` and used to be rewritten to `../..`
        for slug in [".....%2F%2F%2F..", "..%2F..", ".."] {
            assert_eq!(
                status(
                    &w.alice,
                    format!("/api/templates/{}/export_formats/{slug}/assets", w.template)
                )
                .await,
                Status::BadRequest,
                "{slug}"
            );
        }
        Ok(())
    }

    #[sqlx::test]
    async fn templates_are_created_for_own_teams_only(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let create = |team: Uuid| {
            w.alice
                .post("/templates/create")
                .header(ContentType::Form)
                .body(format!("name=New&description=d&team_id={team}"))
                .dispatch()
        };

        assert_eq!(create(w.other_team).await.status(), Status::NotFound);
        assert_eq!(
            templates::list_for_team(&w.pool, w.other_team)
                .await
                .unwrap()
                .len(),
            1
        );

        let response = create(w.team).await;
        assert_eq!(response.status(), Status::SeeOther);
        assert_eq!(
            response.headers().get_one("Location"),
            Some(format!("/templates?team_id={}", w.team).as_str())
        );
        assert_eq!(
            templates::list_for_team(&w.pool, w.team)
                .await
                .unwrap()
                .len(),
            2
        );
        Ok(())
    }

    #[sqlx::test]
    async fn editor_api_rejects_templates_of_other_teams(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        for (id, expected) in [
            (w.template, Status::Ok),
            (w.foreign_template, Status::NotFound),
            (Uuid::new_v4(), Status::NotFound),
        ] {
            assert_eq!(
                status(&w.alice, format!("/api/templates/{id}")).await,
                expected
            );
        }
        let foreign_steps = format!(
            "/api/templates/{}/export_formats/pdf/export_steps",
            w.foreign_template
        );
        assert_eq!(status(&w.alice, foreign_steps).await, Status::NotFound);
        // asset routes check before touching the filesystem
        assert_eq!(
            status(
                &w.alice,
                format!("/api/templates/{}/assets", w.foreign_template)
            )
            .await,
            Status::NotFound
        );
        assert_eq!(
            status(
                &w.alice,
                format!("/api/templates/{}/assets/files/a.css", w.foreign_template)
            )
            .await,
            Status::NotFound
        );

        let update = w
            .alice
            .post(format!("/api/templates/{}", w.foreign_template))
            .header(ContentType::JSON)
            .body(
                serde_json::json!({
                    "id": w.foreign_template, "name": "hijacked", "description": "",
                    "export_formats": {}
                })
                .to_string(),
            )
            .dispatch()
            .await;
        assert_eq!(update.status(), Status::NotFound);
        let foreign = templates::get(&w.pool, w.foreign_template).await.unwrap();
        assert_eq!(foreign.name, "Foreign");

        // the editor saves by posting back what it fetched (including `owner_team_id`)
        let mut own: serde_json::Value = w
            .alice
            .get(format!("/api/templates/{}", w.template))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        own["data"]["name"] = "Renamed".into();
        let save = w
            .alice
            .post(format!("/api/templates/{}", w.template))
            .header(ContentType::JSON)
            .body(own["data"].to_string())
            .dispatch()
            .await;
        assert_eq!(save.status(), Status::Ok);
        let saved = templates::get(&w.pool, w.template).await.unwrap();
        assert_eq!(saved.name, "Renamed");
        assert_eq!(saved.owner_team_id, w.team);
        Ok(())
    }

    #[sqlx::test]
    async fn projects_only_use_templates_of_their_team(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;

        let listed: serde_json::Value = w
            .alice
            .get(format!("/api/projects/{}/templates", w.project))
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        let ids: Vec<&str> = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![w.template.to_string()]);

        let all: serde_json::Value = w
            .alice
            .get("/api/templates")
            .dispatch()
            .await
            .into_json()
            .await
            .unwrap();
        assert!(!all.to_string().contains(&w.foreign_template.to_string()));

        let set = |template: Uuid| {
            w.alice
                .put(format!("/api/projects/{}/template", w.project))
                .header(ContentType::JSON)
                .body(serde_json::json!(template).to_string())
                .dispatch()
        };
        assert_eq!(set(w.foreign_template).await.status(), Status::NotFound);
        assert_eq!(
            projects::get_template_id(&w.pool, w.project).await.unwrap(),
            Some(w.template)
        );
        // PATCH can set the template too
        let patch = w
            .alice
            .patch(format!("/api/projects/{}", w.project))
            .header(ContentType::JSON)
            .body(serde_json::json!({ "template_id": w.foreign_template }).to_string())
            .dispatch()
            .await;
        assert_eq!(patch.status(), Status::NotFound);
        assert_eq!(
            projects::get_template_id(&w.pool, w.project).await.unwrap(),
            Some(w.template)
        );
        let second = templates::insert(&w.pool, w.team, "Second", "")
            .await
            .unwrap();
        assert_eq!(set(second).await.status(), Status::Ok);
        assert_eq!(
            projects::get_template_id(&w.pool, w.project).await.unwrap(),
            Some(second)
        );

        let create = |template: Uuid| {
            w.alice
                .post("/projects/create")
                .header(ContentType::Form)
                .body(format!(
                    "project_name=P&template_id={template}&team_id={}",
                    w.team
                ))
                .dispatch()
        };
        assert_eq!(
            create(w.foreign_template).await.status(),
            Status::BadRequest
        );
        assert_eq!(create(w.template).await.status(), Status::SeeOther);
        Ok(())
    }

    #[sqlx::test]
    async fn templates_without_files_on_disk_dont_crash(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        // a freshly created template has no data/templates/<id>/ folder yet
        let fresh = templates::insert(&w.pool, w.team, "Fresh", "")
            .await
            .unwrap();

        let moved = w
            .alice
            .post(format!("/api/templates/{fresh}/assets/move"))
            .header(ContentType::JSON)
            .body(r#"{"old_path": "a.css", "new_path": "b.css", "overwrite": false}"#)
            .dispatch()
            .await;
        assert_eq!(moved.status(), Status::NotFound);
        let deleted = w
            .alice
            .delete(format!("/api/templates/{fresh}/assets"))
            .header(ContentType::JSON)
            .body(r#"{"paths": ["a.css"]}"#)
            .dispatch()
            .await;
        assert_eq!(deleted.status(), Status::NotFound);

        // renaming an export format without files only updates the database
        templates::insert_export_format(
            &w.pool,
            fresh,
            &vb_exchange::export_formats::ExportFormat {
                slug: "pdf".to_string(),
                name: "PDF".to_string(),
                export_steps: vec![],
                output_files: vec![],
                preview_pdf_path: None,
            },
        )
        .await
        .unwrap();
        let renamed = w
            .alice
            .post(format!(
                "/api/templates/{fresh}/export_formats/pdf/metadata"
            ))
            .header(ContentType::JSON)
            .body(r#"{"slug": "print", "name": "Print", "preview_pdf_path": null}"#)
            .dispatch()
            .await;
        assert_eq!(renamed.status(), Status::Ok);
        let template = templates::get(&w.pool, fresh).await.unwrap();
        assert!(template.export_formats.contains_key("print"));
        Ok(())
    }
}
