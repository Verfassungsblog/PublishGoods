-- Add migration script here
create table api_keys
(
    id         uuid        default gen_random_uuid() not null
        constraint api_keys_pk
            primary key,
    user_id    uuid                                   not null
        constraint api_keys_users_id_fk
            references users
            on delete cascade,
    name       text                                   not null,
    key_prefix text                                   not null
        constraint api_keys_key_prefix_uk
            unique,
    key_hash   text                                   not null,
    created_at timestamptz default now()               not null
);
