use crate::session::session_guard::Session;
use crate::session::session_storage::SessionStorage;
use rocket::response::Redirect;
use rocket::State;

#[get("/logout")]
pub fn logout_page(session: Session, session_storage: &State<SessionStorage>) -> Redirect {
    //Remove the session from the session storage
    session_storage.remove_session(session.id.clone());
    //Redirect to the login page
    Redirect::to("/login")
}
