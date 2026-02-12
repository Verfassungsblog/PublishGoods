use std::collections::HashSet;

pub fn dedup_vec<T: Eq + std::hash::Hash + Clone>(vec: Vec<T>) -> Vec<T> {
    let mut seen = HashSet::new();
    vec.into_iter().filter(|x| seen.insert(x.clone())).collect()
}
