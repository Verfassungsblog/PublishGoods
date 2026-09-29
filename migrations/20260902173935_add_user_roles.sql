-- Add migration script here
create type role as enum ('superadmin', 'admin', 'user');

alter table users
    add role role default 'user'::role not null;