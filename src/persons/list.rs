
use std::sync::Arc;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;
use crate::storage::data_storage::DataStorage;
use vb_exchange::projects::Person;
use crate::session::session_guard::Session;

#[derive(Debug, PartialEq, FromFormField)]
pub enum OrderBy{
    FirstnameAscending,
    FirstnameDescending,
    LastnameAscending,
    LastnameDescending,
}

#[derive(Debug, Serialize)]
struct ListData<'a>{
    persons: Vec<&'a Person>,
    next_offset: Option<u32>,
    previous_offset: Option<u32>,
    offset: u32,
    limit: u32,
}

#[get("/persons?<offset>&<limit>&<order>")]
pub fn list_persons(_session: Session, data_storage: &State<Arc<DataStorage>>, offset: Option<u32>, limit: Option<u32>, order: Option<OrderBy>) -> Template {
    let mut persons : Vec<Person> = data_storage.data.read().unwrap().persons.iter().map(|person|person.1.read().unwrap().clone()).collect();

    let offset = offset.unwrap_or_else(|| 0);
    let limit = limit.unwrap_or_else(|| 10);
    let order = order.unwrap_or_else(|| OrderBy::FirstnameAscending);

    //Sort persons
    match order{
        OrderBy::FirstnameAscending | OrderBy::FirstnameDescending => persons.sort_unstable_by(|a, b| a.first_names.cmp(&b.first_names)),
        OrderBy::LastnameAscending | OrderBy::LastnameDescending => persons.sort_unstable_by(|a, b| a.last_names.cmp(&b.last_names)),
    }

    //Reverse order if descending
    match order{
        OrderBy::FirstnameDescending | OrderBy::LastnameDescending => persons.reverse(),
        _ => {},
    }

    let selected_persons : Vec<&Person> = persons.iter().skip(offset as usize).take(limit as usize).collect();
    let num_of_persons = persons.len() as u32;
    let num_of_pages = (num_of_persons as f32 / limit as f32).ceil() as u32;
    let current_page = (offset / limit) + 1;

    let next_offset = if num_of_pages > current_page {Some(offset + limit)} else {None};
    let previous_offset = if current_page > 1 {Some(offset - limit)} else {None};

    let data = ListData{
        persons: selected_persons,
        next_offset,
        previous_offset,
        offset,
        limit,
    };
    Template::render("persons", data)
}