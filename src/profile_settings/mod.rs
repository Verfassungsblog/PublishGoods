use crate::session::session_guard::Session;
use rocket_dyn_templates::Template;

pub mod teams;

#[get("/profile-settings")]
pub fn profile_settings(_session: Session) -> Template {
    Template::render("profile_settings", ())
}
