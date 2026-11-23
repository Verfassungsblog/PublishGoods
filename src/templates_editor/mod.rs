pub mod api;
pub mod git;
pub mod user_interface;

/// HTTP-level coverage of template ownership: templates are only visible, editable and usable
/// within the team that owns them.
#[cfg(test)]
mod integration_tests {
    use crate::db::repositories::{projects, teams, templates, users};
    use crate::projects::api::get::integration_tests::dummy_settings;
    use crate::session::session_storage::SessionStorage;
    use crate::settings::Settings;
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
        /// settings with a `data_path` of its own, removed on drop (also when the test fails)
        settings: Settings,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.settings.data_path);
        }
    }

    async fn login(pool: &PgPool, settings: &Settings, email: &str) -> Client {
        let client = anonymous(pool, settings).await;
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
    async fn anonymous(pool: &PgPool, settings: &Settings) -> Client {
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool.clone())
            .manage(settings.clone())
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
                    super::api::add_export_format,
                    super::api::delete_export_format,
                    super::api::update_asset_file,
                    super::api::create_file_asset,
                    super::api::create_folder_asset,
                    super::api::create_file_asset_for_export_format,
                    super::api::create_folder_asset_for_export_format,
                    super::api::delete_assets_for_export_format,
                    super::api::update_asset_file_for_export_format,
                    super::api::move_asset_for_export_format,
                    super::api::create_export_step,
                    super::api::move_export_step,
                    super::api::update_export_step,
                    super::api::delete_export_step,
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

        let settings = Settings {
            data_path: std::env::temp_dir()
                .join(format!("vb-templates-test-{}", Uuid::new_v4()))
                .to_string_lossy()
                .into_owned(),
            ..dummy_settings()
        };
        World {
            alice: login(&pool, &settings, "alice@example.com").await,
            settings,
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
        let anonymous = anonymous(&w.pool, &w.settings).await;
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
        let templates = templates::list_for_team(&w.pool, w.team).await.unwrap();
        assert_eq!(templates.len(), 2);
        // the new template got its repository, with an initial commit
        let created = templates.iter().find(|t| t.id != w.template).unwrap();
        assert_eq!(commit_messages(&w, created.id), vec!["Create template"]);
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

    /// The commit messages of a template's repository, newest first.
    fn commit_messages(w: &World, template: Uuid) -> Vec<String> {
        let repo = git2::Repository::open(w.settings.template_git_dir(template)).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.map(|oid| {
            let commit = repo.find_commit(oid.unwrap()).unwrap();
            assert_eq!(commit.author().email(), commit.committer().email());
            commit.message().unwrap().to_string()
        })
        .collect()
    }

    /// The content of `path` in the latest commit of a template's repository.
    fn committed_file(w: &World, template: Uuid, path: &str) -> Option<String> {
        let repo = git2::Repository::open(w.settings.template_git_dir(template)).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let entry = tree.get_path(std::path::Path::new(path)).ok()?;
        let blob = repo.find_blob(entry.id()).unwrap();
        Some(String::from_utf8(blob.content().to_vec()).unwrap())
    }

    #[sqlx::test]
    async fn template_changes_are_committed_to_git(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        let dir = w.settings.template_dir(id);
        let _ = std::fs::remove_dir_all(&dir);

        let added = w
            .alice
            .post(format!("/api/templates/{id}/export_formats"))
            .header(ContentType::JSON)
            .body(r#"{"slug": "pdf", "name": "PDF", "export_steps": [], "output_files": [], "preview_pdf_path": null}"#)
            .dispatch()
            .await;
        assert_eq!(added.status(), Status::Ok);
        // the repository lives next to the template directory, not in it
        assert!(w.settings.template_git_dir(id).is_dir());
        assert!(!dir.join(".git").exists());
        // metadata is versioned as template.json, without volatile or internal fields, and
        // stays out of the directory the renderer gets
        assert!(!dir.join("template.json").exists());
        let json = committed_file(&w, id, "template.json").unwrap();
        assert!(json.contains("\"pdf\""));
        assert!(!json.contains("owner_team_id"));

        // an upload is simulated by writing the file, the routes then edit, move and delete it
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/a.css"), "a").unwrap();
        let updated = w
            .alice
            .put(format!("/api/templates/{id}/assets/files/a.css"))
            .header(ContentType::JSON)
            .body(r#"{"content": "b"}"#)
            .dispatch()
            .await;
        assert_eq!(updated.status(), Status::Ok);
        let moved = w
            .alice
            .post(format!("/api/templates/{id}/assets/move"))
            .header(ContentType::JSON)
            .body(r#"{"old_path": "a.css", "new_path": "b.css", "overwrite": false}"#)
            .dispatch()
            .await;
        assert_eq!(moved.status(), Status::Ok);
        let deleted = w
            .alice
            .delete(format!("/api/templates/{id}/assets"))
            .header(ContentType::JSON)
            .body(r#"{"paths": ["b.css"]}"#)
            .dispatch()
            .await;
        assert_eq!(deleted.status(), Status::Ok);

        // one commit per change (the simulated upload is part of the first asset commit), newest first
        let messages = commit_messages(&w, id);
        assert_eq!(
            messages,
            vec![
                "Delete assets/b.css",
                "Move asset assets/a.css to assets/b.css",
                "Update asset assets/a.css",
                "Add export format pdf",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );

        // renaming the export format changes template.json only
        let renamed = w
            .alice
            .post(format!("/api/templates/{id}/export_formats/pdf/metadata"))
            .header(ContentType::JSON)
            .body(r#"{"slug": "print", "name": "Print", "preview_pdf_path": null}"#)
            .dispatch()
            .await;
        assert_eq!(renamed.status(), Status::Ok);
        assert_eq!(commit_messages(&w, id)[0], "Update export format print");
        assert!(
            committed_file(&w, id, "template.json")
                .unwrap()
                .contains("\"print\"")
        );

        // dot-git names can't be uploaded or used as slugs
        let reserved = w
            .alice
            .post(format!("/api/templates/{id}/export_formats"))
            .header(ContentType::JSON)
            .body(r#"{"slug": ".git", "name": "x", "export_steps": [], "output_files": [], "preview_pdf_path": null}"#)
            .dispatch()
            .await;
        assert_eq!(reserved.status(), Status::BadRequest);

        // ... also not if sanitizing is what turns a name into one (`.g!it` -> `.git`)
        std::fs::write(dir.join("assets/c.css"), "c").unwrap();
        for new_path in [".g!it", ".git."] {
            let moved = w
                .alice
                .post(format!("/api/templates/{id}/assets/move"))
                .header(ContentType::JSON)
                .body(format!(
                    r#"{{"old_path": "c.css", "new_path": "{new_path}", "overwrite": false}}"#
                ))
                .dispatch()
                .await;
            assert_eq!(moved.status(), Status::BadRequest, "{new_path}");
        }
        assert!(dir.join("assets/c.css").exists());

        // other names git treats specially are ordinary files, and are versioned
        let moved = send(
            &w,
            "POST",
            format!("/api/templates/{id}/assets/move"),
            Some(serde_json::json!({
                "old_path": "c.css", "new_path": ".gitignore", "overwrite": false
            })),
        )
        .await;
        assert_eq!(moved, Status::Ok);
        let repo = git2::Repository::open(w.settings.template_git_dir(id)).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(
            tree.get_path(std::path::Path::new("assets/.gitignore"))
                .is_ok()
        );

        // deleting a format that has no folder still succeeds and is recorded (the folder
        // `add_export_format` created, and the rename moved, is removed first)
        std::fs::remove_dir_all(dir.join("formats/print")).unwrap();
        let deleted = w
            .alice
            .delete(format!("/api/templates/{id}/export_formats/print"))
            .dispatch()
            .await;
        assert_eq!(deleted.status(), Status::Ok);
        assert_eq!(commit_messages(&w, id)[0], "Delete export format print");
        assert!(
            !committed_file(&w, id, "template.json")
                .unwrap()
                .contains("\"print\"")
        );
        assert!(!dir.join("template.json").exists());

        Ok(())
    }

    async fn version(w: &World, template: Uuid) -> Uuid {
        templates::get(&w.pool, template).await.unwrap().version
    }

    /// A `multipart/form-data` body with one file `name` as the `file` field.
    fn upload(name: &str, content: &str) -> (ContentType, String) {
        let body = format!(
            "--XBOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
             Content-Type: text/plain\r\n\r\n{content}\r\n--XBOUNDARY--\r\n"
        );
        let content_type =
            ContentType::new("multipart", "form-data").with_params(("boundary", "XBOUNDARY"));
        (content_type, body)
    }

    /// Sends a JSON `body` (or none) to `path` with the given method, and returns the status.
    async fn send(
        w: &World,
        method: &str,
        path: String,
        body: Option<serde_json::Value>,
    ) -> Status {
        let request = match method {
            "POST" => w.alice.post(path),
            "PUT" => w.alice.put(path),
            "DELETE" => w.alice.delete(path),
            _ => unreachable!(),
        };
        let request = match body {
            Some(body) => request.header(ContentType::JSON).body(body.to_string()),
            None => request,
        };
        request.dispatch().await.status()
    }

    async fn add_format(w: &World, slug: &str) {
        let body = serde_json::json!({
            "slug": slug, "name": slug, "export_steps": [], "output_files": [],
            "preview_pdf_path": null
        });
        let path = format!("/api/templates/{}/export_formats", w.template);
        assert_eq!(send(w, "POST", path, Some(body)).await, Status::Ok);
    }

    #[sqlx::test]
    async fn failed_deletes_record_only_what_was_deleted(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        add_format(&w, "pdf").await;
        for dir in ["assets", "formats/pdf"] {
            std::fs::create_dir_all(w.settings.template_dir(id).join(dir)).unwrap();
            std::fs::write(w.settings.template_dir(id).join(dir).join("a.css"), "a").unwrap();
        }

        for (url, prefix) in [
            (format!("/api/templates/{id}/assets"), "assets"),
            (
                format!("/api/templates/{id}/export_formats/pdf/assets"),
                "formats/pdf",
            ),
        ] {
            // nothing deleted: no version bump, no commit (also for unusable paths)
            let (version_before, commits_before) = (version(&w, id).await, commit_messages(&w, id));
            for paths in [
                serde_json::json!(["missing.css"]),
                serde_json::json!(["../x"]),
            ] {
                let status = send(
                    &w,
                    "DELETE",
                    url.clone(),
                    Some(serde_json::json!({ "paths": paths })),
                )
                .await;
                assert_ne!(status, Status::Ok);
            }
            assert_eq!(version(&w, id).await, version_before, "{prefix}");
            assert_eq!(commit_messages(&w, id), commits_before, "{prefix}");

            // deleted until the failure: recorded, naming only the deleted file
            let status = send(
                &w,
                "DELETE",
                url,
                Some(serde_json::json!({ "paths": ["a.css", "missing.css"] })),
            )
            .await;
            assert_ne!(status, Status::Ok);
            assert_ne!(version(&w, id).await, version_before, "{prefix}");
            assert_eq!(commit_messages(&w, id)[0], format!("Delete {prefix}/a.css"));
        }
        Ok(())
    }

    #[sqlx::test]
    async fn deletes_keep_their_statuses_and_bump_the_version(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        add_format(&w, "pdf").await;
        for dir in ["assets", "formats/pdf"] {
            std::fs::create_dir_all(w.settings.template_dir(id).join(dir)).unwrap();
        }
        for (url, invalid) in [
            (format!("/api/templates/{id}/assets"), Status::ImATeapot),
            (
                format!("/api/templates/{id}/export_formats/pdf/assets"),
                Status::BadRequest,
            ),
        ] {
            // nothing to delete is still a (successful) change
            let before = version(&w, id).await;
            let body = serde_json::json!({ "paths": [] });
            assert_eq!(
                send(&w, "DELETE", url.clone(), Some(body)).await,
                Status::Ok
            );
            assert_ne!(version(&w, id).await, before, "{url}");

            let body = serde_json::json!({ "paths": [""] });
            assert_eq!(send(&w, "DELETE", url, Some(body)).await, invalid);
        }
        Ok(())
    }

    #[sqlx::test]
    async fn only_created_folders_are_changes(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        add_format(&w, "pdf").await;
        std::fs::create_dir_all(w.settings.template_dir(id).join("formats/pdf")).unwrap();
        std::fs::create_dir_all(w.settings.template_dir(id).join("assets")).unwrap();

        for (url, dir, conflict) in [
            (
                format!("/api/templates/{id}/assets/folder"),
                "assets/sub",
                Status::ImATeapot,
            ),
            (
                format!("/api/templates/{id}/export_formats/pdf/assets/folder"),
                "formats/pdf/sub",
                Status::Conflict,
            ),
        ] {
            let body = serde_json::json!({ "name": "sub" });
            let before = version(&w, id).await;
            assert_eq!(
                send(&w, "POST", url.clone(), Some(body.clone())).await,
                Status::Ok
            );
            assert!(w.settings.template_dir(id).join(dir).is_dir());
            let created = version(&w, id).await;
            assert_ne!(created, before, "{dir}");
            // git doesn't track the empty folder: no commit for it
            assert!(!commit_messages(&w, id).iter().any(|m| m.contains("folder")));

            assert_eq!(send(&w, "POST", url, Some(body)).await, conflict);
            assert_eq!(
                version(&w, id).await,
                created,
                "{dir}: failed create bumped version"
            );
        }
        // names that can't be folders are rejected
        for name in ["", ".git"] {
            let url = format!("/api/templates/{id}/assets/folder");
            let status = send(&w, "POST", url, Some(serde_json::json!({ "name": name }))).await;
            assert_eq!(status, Status::BadRequest, "{name:?}");
        }
        Ok(())
    }

    #[sqlx::test]
    async fn export_format_metadata_update_bumps_version(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        add_format(&w, "pdf").await;
        let before = version(&w, id).await;
        let url = format!("/api/templates/{id}/export_formats/pdf/metadata");
        let body =
            serde_json::json!({ "slug": "pdf", "name": "Renamed", "preview_pdf_path": null });
        assert_eq!(send(&w, "POST", url, Some(body)).await, Status::Ok);
        assert_ne!(version(&w, id).await, before);
        assert_eq!(commit_messages(&w, id)[0], "Update export format pdf");
        Ok(())
    }

    #[sqlx::test]
    async fn every_change_route_commits(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        let api = format!("/api/templates/{id}");
        add_format(&w, "pdf").await;
        // the folders an upload goes to exist already
        for dir in ["assets", "formats/pdf"] {
            std::fs::create_dir_all(w.settings.template_dir(id).join(dir)).unwrap();
        }
        let latest = || commit_messages(&w, id).remove(0);

        // update_template
        let body = serde_json::json!({
            "id": id, "name": "Renamed book", "description": "",
            "export_formats": { "pdf": {
                "slug": "pdf", "name": "pdf", "export_steps": [], "output_files": [],
                "preview_pdf_path": null
            } }
        });
        assert_eq!(send(&w, "POST", api.clone(), Some(body)).await, Status::Ok);
        assert_eq!(latest(), "Update template");

        // file uploads, a taken name gets a number (the commit names the file as saved; the
        // dot before the number is how the upload routes number files)
        for (url, name, expected) in [
            (
                format!("{api}/assets/file"),
                "up.css",
                "Add asset assets/up.css",
            ),
            (
                format!("{api}/assets/file"),
                "up.css",
                "Add asset assets/up._1.css",
            ),
            (
                format!("{api}/export_formats/pdf/assets/file"),
                "f.txt",
                "Add asset formats/pdf/f.txt",
            ),
        ] {
            let (content_type, body) = upload(name, "text");
            let response = w
                .alice
                .post(url)
                .header(content_type)
                .body(body)
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::Ok, "{name}");
            assert_eq!(latest(), expected);
        }

        // edit, move and delete a file of an export format
        let formats = format!("{api}/export_formats/pdf");
        let status = send(
            &w,
            "PUT",
            format!("{formats}/assets/files/f.txt"),
            Some(serde_json::json!({ "content": "new" })),
        )
        .await;
        assert_eq!(status, Status::Ok);
        assert_eq!(latest(), "Update asset formats/pdf/f.txt");
        let moved =
            serde_json::json!({ "old_path": "f.txt", "new_path": "g.txt", "overwrite": false });
        let status = send(&w, "POST", format!("{formats}/assets/move"), Some(moved)).await;
        assert_eq!(status, Status::Ok);
        assert_eq!(
            latest(),
            "Move asset formats/pdf/f.txt to formats/pdf/g.txt"
        );
        let status = send(
            &w,
            "DELETE",
            format!("{formats}/assets"),
            Some(serde_json::json!({ "paths": ["g.txt"] })),
        )
        .await;
        assert_eq!(status, Status::Ok);
        assert_eq!(latest(), "Delete formats/pdf/g.txt");

        // export steps
        let step = |id: Option<String>, name: &str| {
            serde_json::json!({
                "id": id, "name": name, "files_to_keep": [],
                "data": { "Raw": { "entry_point": "main.hbs", "output_file": "main.html" } }
            })
        };
        let created = w
            .alice
            .post(format!("{formats}/export_steps"))
            .header(ContentType::JSON)
            .body(step(None, "one").to_string())
            .dispatch()
            .await;
        assert_eq!(created.status(), Status::Ok);
        let first: serde_json::Value = created.into_json().await.unwrap();
        let first = first["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(latest(), "Add export step to format pdf");
        let second = w
            .alice
            .post(format!("{formats}/export_steps"))
            .header(ContentType::JSON)
            .body(step(None, "two").to_string())
            .dispatch()
            .await;
        let second: serde_json::Value = second.into_json().await.unwrap();
        let second = second["data"]["id"].as_str().unwrap().to_string();

        let status = send(
            &w,
            "POST",
            format!("{formats}/export_steps/{second}/move"),
            Some(serde_json::json!({ "move_after": null })),
        )
        .await;
        assert_eq!(status, Status::Ok);
        assert_eq!(latest(), "Reorder export steps of format pdf");
        let status = send(
            &w,
            "POST",
            format!("{formats}/export_steps/{first}"),
            Some(step(Some(first.clone()), "renamed")),
        )
        .await;
        assert_eq!(status, Status::Ok);
        assert_eq!(latest(), "Update export step of format pdf");
        let status = send(
            &w,
            "DELETE",
            format!("{formats}/export_steps/{first}"),
            None,
        )
        .await;
        assert_eq!(status, Status::Ok);
        assert_eq!(latest(), "Delete export step from format pdf");

        // nothing but template files was written into the template directory
        assert!(!w.settings.template_dir(id).join(".git").exists());
        Ok(())
    }

    #[sqlx::test]
    async fn a_template_created_while_its_team_is_deleted_is_reported_or_rejected(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let w = world(pool).await;
        let mut tx = w.pool.begin().await?;
        let listed = templates::list_ids_for_teams_being_deleted(&mut tx, &[w.team])
            .await
            .unwrap();
        assert_eq!(listed, vec![w.template]);

        // created after the listing, before the team is gone
        let pool = w.pool.clone();
        let team = w.team;
        let late = tokio::spawn(async move { templates::insert(&pool, team, "Late", "").await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!late.is_finished(), "not blocked by the team being deleted");

        sqlx::query("DELETE FROM teams WHERE id = $1")
            .bind(w.team)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        // it never exists, so nothing is left to clean up
        assert!(late.await.unwrap().is_err());
        Ok(())
    }

    #[sqlx::test]
    async fn an_export_format_without_a_folder_isnt_kept(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        let dir = w.settings.template_dir(id);
        // a file where the formats directory should be: no format folder can be created
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("formats"), "").unwrap();
        let body = r#"{"slug": "pdf", "name": "PDF", "export_steps": [], "output_files": [], "preview_pdf_path": null}"#;
        let add = || {
            w.alice
                .post(format!("/api/templates/{id}/export_formats"))
                .header(ContentType::JSON)
                .body(body)
                .dispatch()
        };

        let before = version(&w, id).await;
        assert_eq!(add().await.status(), Status::InternalServerError);
        let template = templates::get(&w.pool, id).await.unwrap();
        assert!(template.export_formats.is_empty());
        assert_eq!(template.version, before);

        // nothing is in the way of trying again
        std::fs::remove_file(dir.join("formats")).unwrap();
        assert_eq!(add().await.status(), Status::Ok);
        assert!(dir.join("formats/pdf").is_dir());
        Ok(())
    }

    #[cfg(unix)]
    #[sqlx::test]
    async fn an_export_format_whose_folder_cant_be_deleted_is_kept(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let w = world(pool).await;
        let id = w.template;
        let dir = w.settings.template_dir(id);
        templates::insert_export_format(
            &w.pool,
            id,
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
        // a folder whose content can't be deleted
        let locked = dir.join("formats/pdf/locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("a.css"), "a").unwrap();
        let set_mode = |mode| std::fs::set_permissions(&locked, PermissionsExt::from_mode(mode));
        set_mode(0o500).unwrap();
        // permissions don't apply to root
        let enforced = std::fs::File::create(locked.join("probe")).is_err();

        let delete = || {
            w.alice
                .delete(format!("/api/templates/{id}/export_formats/pdf"))
                .dispatch()
        };
        let failed = delete().await.status();
        set_mode(0o700).unwrap();
        if !enforced {
            return Ok(());
        }
        assert_eq!(failed, Status::InternalServerError);
        let template = templates::get(&w.pool, id).await.unwrap();
        assert!(template.export_formats.contains_key("pdf"));

        // repeating the request works
        assert_eq!(delete().await.status(), Status::Ok);
        assert!(!dir.join("formats/pdf").exists());
        let template = templates::get(&w.pool, id).await.unwrap();
        assert!(template.export_formats.is_empty());
        Ok(())
    }

    #[sqlx::test]
    async fn deleting_an_unknown_export_format_leaves_files_alone(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        let stray = w.settings.template_dir(id).join("formats/pdf");
        std::fs::create_dir_all(&stray).unwrap();
        let deleted = w
            .alice
            .delete(format!("/api/templates/{id}/export_formats/pdf"))
            .dispatch()
            .await;
        assert_eq!(deleted.status(), Status::NotFound);
        assert!(stray.is_dir());
        Ok(())
    }

    #[sqlx::test]
    async fn existing_templates_are_imported_once(pool: PgPool) -> sqlx::Result<()> {
        let w = world(pool).await;
        let id = w.template;
        let workdir = w.settings.template_dir(id);
        std::fs::create_dir_all(workdir.join("assets")).unwrap();
        std::fs::write(workdir.join("assets/a.css"), "a").unwrap();

        super::git::import_existing(&w.pool, &w.settings).await;
        assert_eq!(commit_messages(&w, id), vec!["Initial import"]);
        // no files, no repository
        assert!(!w.settings.template_git_dir(w.foreign_template).exists());
        assert!(!w.settings.template_dir(w.foreign_template).exists());

        // already imported: not committed again, even with changes pending; a stale lock goes
        std::fs::write(workdir.join("assets/b.css"), "b").unwrap();
        let lock = w.settings.template_git_dir(id).join("index.lock");
        std::fs::write(&lock, "").unwrap();
        super::git::import_existing(&w.pool, &w.settings).await;
        assert!(!lock.exists());
        assert_eq!(commit_messages(&w, id), vec!["Initial import"]);

        // and committing works again afterwards, picking up the pending change
        let lock = super::git::lock(id).await;
        super::git::commit(&w.pool, &w.settings, id, lock, "After import").await;
        assert_eq!(commit_messages(&w, id)[0], "After import");
        Ok(())
    }
}
