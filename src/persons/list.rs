use crate::db::repositories::persons;
use crate::db::repositories::teams;
use crate::permissions::{ResourceType, has_permission};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use rocket_dyn_templates::Template;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;
use vb_exchange::projects::Person;

#[derive(Debug, PartialEq, FromFormField)]
pub enum OrderBy {
    FirstnameAscending,
    FirstnameDescending,
    LastnameAscending,
    LastnameDescending,
}

impl From<OrderBy> for persons::PersonOrderBy {
    fn from(order: OrderBy) -> Self {
        match order {
            OrderBy::FirstnameAscending => persons::PersonOrderBy::FirstnameAscending,
            OrderBy::FirstnameDescending => persons::PersonOrderBy::FirstnameDescending,
            OrderBy::LastnameAscending => persons::PersonOrderBy::LastnameAscending,
            OrderBy::LastnameDescending => persons::PersonOrderBy::LastnameDescending,
        }
    }
}

#[derive(Debug, Serialize)]
struct ListData {
    teams: Vec<teams::TeamOption>,
    selected_team_id: Uuid,
    persons: Vec<Person>,
    next_offset: Option<u32>,
    previous_offset: Option<u32>,
    offset: u32,
    limit: u32,
}

/// Renders a paginated, sortable listing of one team's person library. Sorting and pagination
/// are performed by the database (`ORDER BY` / `LIMIT` / `OFFSET`) according to the `order`,
/// `offset`, and `limit` query parameters (defaults: ascending by first name, offset 0,
/// limit 10).
///
/// `team_id` selects the team (it must be one the user belongs to, else 404); without it the
/// user's `Default` team is shown, or their first team if they aren't in it.
#[get("/persons?<offset>&<limit>&<order>&<team_id>")]
pub async fn list_persons(
    session: Session,
    pool: &State<PgPool>,
    offset: Option<u32>,
    limit: Option<u32>,
    order: Option<OrderBy>,
    team_id: Option<String>,
) -> Result<Template, Status> {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(10).max(1);
    let order = order.unwrap_or(OrderBy::FirstnameAscending);

    let user_teams = teams::list_for_user(pool.inner(), session.user_id)
        .await
        .map_err(|_| Status::InternalServerError)?;

    let selected_team_id = match team_id {
        Some(team_id) => {
            let team_id = Uuid::parse_str(&team_id).map_err(|_| Status::BadRequest)?;
            has_permission(pool, ResourceType::Team, team_id, session.user_id, None)
                .await
                .map_err(|_| Status::NotFound)?;
            team_id
        }
        None => teams::preferred_team(&user_teams)
            .map(|t| t.id)
            .ok_or(Status::NotFound)?,
    };

    let selected_persons = persons::list_for_team_paginated(
        pool.inner(),
        selected_team_id,
        order.into(),
        offset as i64,
        limit as i64,
    )
    .await
    .map_err(|_| Status::InternalServerError)?;
    let num_of_persons = persons::count_for_team(pool.inner(), selected_team_id)
        .await
        .map_err(|_| Status::InternalServerError)? as u32;
    let num_of_pages = (num_of_persons as f32 / limit as f32).ceil() as u32;
    let current_page = (offset / limit) + 1;

    let next_offset = if num_of_pages > current_page {
        Some(offset + limit)
    } else {
        None
    };
    let previous_offset = if current_page > 1 {
        Some(offset.saturating_sub(limit))
    } else {
        None
    };

    let data = ListData {
        teams: teams::team_options(&user_teams, selected_team_id),
        selected_team_id,
        persons: selected_persons,
        next_offset,
        previous_offset,
        offset,
        limit,
    };
    Ok(Template::render("persons", data))
}
