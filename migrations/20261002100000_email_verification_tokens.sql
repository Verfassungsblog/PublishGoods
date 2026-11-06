-- Pending email verification: argon2 hash of the mailed token and its expiry.
ALTER TABLE users
    ADD COLUMN email_verification_token_hash text,
    ADD COLUMN email_verification_token_valid_until timestamptz;
