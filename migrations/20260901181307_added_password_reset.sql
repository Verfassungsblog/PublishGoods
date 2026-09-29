-- Add migration script here
alter table users
    add password_reset_token_hash text;

alter table users
    add password_reset_token_valid_until timestamptz;

