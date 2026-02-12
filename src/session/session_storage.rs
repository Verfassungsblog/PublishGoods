use crate::session::session_guard::Session;
use rand::distributions::Alphanumeric;
use rand::distributions::Distribution;
use rand::thread_rng;
use rocket::serde::Deserialize;
use std::collections::HashMap;
use std::ops::Add;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct SessionStorage {
    sessions: RwLock<HashMap<String, Session>>,
}

impl SessionStorage {
    pub fn new() -> SessionStorage {
        SessionStorage {
            sessions: Default::default(),
        }
    }

    pub fn remove_session(&self, id: String) {
        self.sessions.write().unwrap().remove(&id);
    }

    pub fn get_session(&self, id: String, refresh_valid_until: bool) -> Option<Session> {
        if self.sessions.read().unwrap().contains_key(&id) {
            let session = self.sessions.read().unwrap().get(&id).unwrap().clone();
            return if SystemTime::now() > session.valid_until {
                self.sessions.write().unwrap().remove(&id);
                println!("Session {} expired.", id);
                None
            } else {
                if refresh_valid_until {
                    self.sessions
                        .write()
                        .unwrap()
                        .get_mut(&id)
                        .unwrap()
                        .valid_until = SystemTime::now().add(Duration::new(1800, 0));
                }
                Some(session)
            };
        } else {
            println!("Session {} not found in SessionStorage.", id);
            None
        }
    }
    pub fn generate_session(&self, user_email: String, user_id: uuid::Uuid) -> Session {
        let mut session_id = String::new();

        while session_id.is_empty() {
            let rstr = generate_random_string();
            if !self.sessions.read().unwrap().contains_key(&*rstr) {
                session_id = rstr;
            }
        }

        let session = Session {
            id: session_id.clone(),
            user_id,
            valid_until: SystemTime::now().add(Duration::new(1800, 0)),
            user_email,
        };

        self.sessions
            .write()
            .unwrap()
            .insert(session_id, session.clone());

        session
    }
}

fn generate_random_string() -> String {
    let mut rng = thread_rng();
    Alphanumeric
        .sample_iter(&mut rng)
        .take(32)
        .map(char::from)
        .collect()
}
