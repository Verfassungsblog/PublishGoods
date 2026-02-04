use crate::storage::project_storage::sections::Section;

/// Generates a block id with 10 random characters which is unique for the section
pub fn generate_id(section: &Section) -> String {
    unimplemented!();
    /*
    let existing_ids: Vec<String> = section
        .children
        .iter()
        .map(|child| child.id.clone())
        .collect();

    loop {
        let rand_id: String = thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .map(char::from)
            .take(10)
            .collect();
        if !existing_ids.contains(&rand_id) {
            return rand_id;
        }
    }

     */
}
