use crate::db::repositories::persons;
use crate::session::session_guard::Session;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;
use sqlx::PgPool;
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
    persons: Vec<Person>,
    next_offset: Option<u32>,
    previous_offset: Option<u32>,
    offset: u32,
    limit: u32,
}

/// Renders a paginated, sortable listing of all persons. Sorting and pagination are
/// performed by the database (`ORDER BY` / `LIMIT` / `OFFSET`) according to the `order`,
/// `offset`, and `limit` query parameters (defaults: ascending by first name, offset 0,
/// limit 10).
#[get("/persons?<offset>&<limit>&<order>")]
pub async fn list_persons(
    _session: Session,
    pool: &State<PgPool>,
    offset: Option<u32>,
    limit: Option<u32>,
    order: Option<OrderBy>,
) -> Template {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(10);
    let order = order.unwrap_or(OrderBy::FirstnameAscending);

    let selected_persons =
        persons::list_paginated(pool.inner(), order.into(), offset as i64, limit as i64)
            .await
            .unwrap_or_default();
    let num_of_persons = persons::count(pool.inner()).await.unwrap_or(0) as u32;
    let num_of_pages = (num_of_persons as f32 / limit as f32).ceil() as u32;
    let current_page = (offset / limit) + 1;

    let next_offset = if num_of_pages > current_page {
        Some(offset + limit)
    } else {
        None
    };
    let previous_offset = if current_page > 1 {
        Some(offset - limit)
    } else {
        None
    };

    let data = ListData {
        persons: selected_persons,
        next_offset,
        previous_offset,
        offset,
        limit,
    };
    Template::render("persons", data)
}
