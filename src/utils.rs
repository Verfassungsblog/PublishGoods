pub mod api_helpers;
pub mod block_id_generator;
pub mod csl;
pub mod dedup;
pub mod lobid_proxy;
pub mod timeout;

/// Compares two byte strings without short-circuiting on the first differing byte.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Hashes a plaintext password with Argon2 using a freshly generated salt.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    use argon2::PasswordHasher;
    let salt =
        argon2::password_hash::SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Ok(argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}
