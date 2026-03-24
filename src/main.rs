//! Verfassungsbooks serves as a web application for the creation of books including
//! import and export from various formats.
//!
//! # Settings
//! You have to create a new configuration file in the config folder to change the default settings.
//! The default settings pub(crate)are stored in the file config/default.toml, create a new file named "local.toml" in the same folder.

// #![warn(missing_docs)]
// #![warn(clippy::missing_docs_in_private_items)]

#[macro_use]
extern crate rocket;
use crate::projects::websocket::WebsocketManager;
//noinspection RsMainFunctionNotFound
use crate::session::session_storage::SessionStorage;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::data_storage::current::User;
use crate::storage::project_storage::ProjectStorage;
use crate::storage::{data_storage, save_data_worker};
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use crate::utils::csl::CslData;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHasher};
use log::{debug, info};
use rand::Rng;
use rocket::Request;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use std::sync::Arc;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use vb_exchange::certs::{load_client_cert, load_crl, load_private_key, load_root_ca};

pub mod cleaner;
pub mod export;
pub mod import;
pub mod persons;
pub mod projects;
pub mod session;
mod settings;
pub mod settings_page;
pub mod storage;
pub mod templates_editor;
pub mod utils;

/// This is the catch-all route that redirects all 401 errors to the login page.
#[catch(401)]
fn forward_to_login(req: &Request) -> Result<Redirect, ApiError> {
    if req.uri().path().starts_with("/api/") {
        Err(ApiErrorType::Unauthorized.into())
    } else {
        Ok(Redirect::to("/login"))
    }
}

/// Starts the web server, mounts all routes and attaches the [SessionStorage][session::session_storage::SessionStorage] and [Settings][settings::Settings] structs.
#[launch]
async fn rocket() -> _ {
    env_logger::init();
    debug!("Initialized Logger, starting application.");

    let settings = Settings::builder().unwrap();

    //Check if data directory exists, if not create it
    if !std::path::Path::new(&format!("{}/projects", settings.data_path)).exists() {
        info!("Data directory does not exist, creating it...");
        std::fs::create_dir_all(format!("{}/projects", settings.data_path)).unwrap(); //Intentionally panic if directory creation fails
        //Create empty DataStorage
        info!("Creating empty data storage...");
        let data_storage = data_storage::DataStorage::new();
        //Create new admin user
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
        const PASSWORD_CHARACTERS: [char; 92] = [
            'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q',
            'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
            'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y',
            'Z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '!', '@', '#', '$', '%', '^',
            '&', '*', '(', ')', '_', '+', '-', '=', '[', ']', '{', '}', '|', '\\', ';', ':', '\'',
            '"', ',', '.', '<', '>', '/', '?',
        ];
        let mut password: String = "".to_string();
        {
            let mut random = rand::thread_rng();
            password = (0..20)
                .map(|_| PASSWORD_CHARACTERS[random.gen_range(0..PASSWORD_CHARACTERS.len())])
                .collect();
        }
        let user = User {
            id: uuid::Uuid::new_v4(),
            name: String::from("default"),
            email: String::from("default@default"),
            password_hash: Argon2::default()
                .hash_password(password.as_bytes(), &salt)
                .unwrap()
                .to_string(),
            locked_until: None,
            login_attempts: Vec::new(),
        };
        data_storage.insert_user(user, &settings).await.unwrap();
        data_storage.save_to_disk(&settings).await.unwrap();
        info!(
            "Created new default admin user 'default@default' with password '{}'",
            password
        );
    }

    // Clear temp directory
    let path = format!("{}/temp", settings.data_path);
    let temp_dir = std::path::Path::new(&path);
    if temp_dir.exists() {
        std::fs::remove_dir_all(temp_dir).unwrap();
    }
    std::fs::create_dir(temp_dir).unwrap();

    info!("Loading data storage...");
    let data_storage = Arc::new(DataStorage::load_from_disk(&settings).await.unwrap());
    info!("Scanning project directory for removed/added projects");
    data_storage
        .scan_for_missing_or_deleted_projects(&settings)
        .await
        .unwrap();
    info!("Loading project storage...");
    let project_storage = Arc::new(ProjectStorage::new());

    info!("Loading Citation Locale Files & Styles...");
    let csl_data = Arc::new(CslData::new(&settings));

    info!("Starting auto-save worker...");
    // Start seperate thread for auto-saving
    save_data_worker(
        data_storage.clone(),
        project_storage.clone(),
        settings.clone(),
    )
    .await;

    info!("Starting cleanup worker...");
    cleaner::worker();

    let root_ca = Arc::new(load_root_ca(settings.ca_cert_path.clone()));
    let client_cert = load_client_cert(settings.client_cert_path.clone());
    let client_key = load_private_key(settings.client_key_path.clone());
    let client_key2 = load_private_key(settings.client_key_path.clone());
    let crls = load_crl(settings.revocation_list_path.clone());
    let client_verifier = WebPkiClientVerifier::builder(root_ca.clone())
        .with_crls(crls)
        .build()
        .expect("Couldn't build Client Verifier. Check Certs & Key!");
    let client_config =
        ClientConfig::builder_with_protocol_versions(&[&tokio_rustls::rustls::version::TLS13])
            .with_root_certificates(root_ca)
            .with_client_auth_cert(client_cert, client_key2)
            .expect("Couldn't build Client Config. Check Certs & Key!");

    info!("Starting rendering worker...");
    let rendering_manager = export::rendering_manager::RenderingManager::start(
        settings.clone(),
        data_storage.clone(),
        project_storage.clone(),
        csl_data.clone(),
        Arc::new(client_config),
    );

    info!("Starting import processing worker...");
    let import_manager =
        import::processing::ImportProcessor::start(settings.clone(), project_storage.clone());

    let websocket_manager = Arc::new(WebsocketManager::new(
        project_storage.clone(),
        Arc::new(settings.clone()),
    ));

    info!("Starting web server...");
    rocket::build()
        .register("/", catchers![forward_to_login])
        .attach(Template::fairing())
        .mount("/css", rocket::fs::FileServer::from("static/css"))
        .mount("/js", rocket::fs::FileServer::from("static/js"))
        .mount("/assets", rocket::fs::FileServer::from("static/img"))
        .mount(
            "/",
            routes![
                templates_editor::user_interface::list_templates,
                templates_editor::user_interface::create_template,
                templates_editor::user_interface::form_create_template,
                templates_editor::user_interface::get_template,
                templates_editor::api::get_template,
                templates_editor::api::update_template,
                templates_editor::api::get_assets,
                templates_editor::api::create_folder_asset,
                templates_editor::api::create_file_asset,
                templates_editor::api::move_asset,
                templates_editor::api::delete_assets,
                templates_editor::api::get_asset_file,
                templates_editor::api::update_asset_file,
                templates_editor::api::add_export_format,
                templates_editor::api::delete_export_format,
                templates_editor::api::get_assets_for_export_format,
                templates_editor::api::get_asset_file_for_export_format,
                templates_editor::api::create_file_asset_for_export_format,
                templates_editor::api::delete_assets_for_export_format,
                templates_editor::api::create_folder_asset_for_export_format,
                templates_editor::api::move_asset_for_export_format,
                templates_editor::api::update_asset_file_for_export_format,
                templates_editor::api::get_export_steps,
                templates_editor::api::delete_export_step,
                templates_editor::api::update_export_step,
                templates_editor::api::create_export_step,
                templates_editor::api::update_export_format_metadata,
                templates_editor::api::move_export_step,
                export::api::add_local_rendering_request,
                export::api::get_request_result,
                export::api::get_request_status,
                export::api::get_request_result_specific_file,
                import::api::get_wordpress_categories,
                import::api::get_wordpress_posts_preview,
                utils::lobid_proxy::search_gnd,
                session::logout::logout_page,
                session::login::login_page,
                session::login::process_login_form,
                projects::create::show_create_project,
                projects::api::delete_project_upload,
                projects::create::process_create_project,
                projects::list::list_projects,
                projects::editor::show_editor,
                projects::api::get::get_project,
                projects::api::get_project_template,
                projects::api::set_project_template,
                projects::api::patch::patch_project,
                projects::api::list_templates,
                projects::api::delete_project,
                persons::api::delete_person,
                persons::list::list_persons,
                persons::create::show_create_person,
                persons::api::create_person,
                persons::api::get_person,
                persons::api::update_person,
                persons::api::search_persons,
                projects::api::get_project_contents,
                projects::api::add_content,
                projects::api::move_content_after,
                projects::api::move_content_child_of,
                projects::api::sections::get_section,
                projects::api::sections::update_section,
                projects::api::sections::delete_section,
                projects::api::sections::move_section_after,
                projects::api::sections::move_section_child_of,
                projects::api::bibliography::get_bibliography_tree,
                projects::api::bibliography::search_bibliography_entries,
                projects::api::bibliography::get_bibliography_entry,
                projects::api::bibliography::post_bibliography_entry,
                projects::api::bibliography::patch_bibliography_entry,
                projects::api::bibliography::delete_bibliography_entry,
                projects::api::upload_to_project,
                import::api::poll_import_status,
                projects::api::get_project_upload,
                import::api::import_from_wordpress,
                export::download::download_rendering,
                settings_page::settings_page,
                settings_page::api::add_user,
                settings_page::api::update_user,
                settings_page::api::delete_user,
                import::api::import_from_upload,
                projects::websocket::websocket,
            ],
        )
        .manage(SessionStorage::new())
        .manage(settings)
        .manage(data_storage)
        .manage(project_storage)
        .manage(import_manager)
        .manage(csl_data)
        .manage(rendering_manager)
        .manage(websocket_manager)
}

//TODO: clean shutdown
