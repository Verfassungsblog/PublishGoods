-- Add migration script here
create table invitations
(
    id        uuid        default gen_random_uuid()   not null
        constraint invitations_pk
            primary key,
    team_id   uuid                                    not null
        constraint invitations_teams_id_fk
            references teams
            on delete cascade,
    role      team_role   default 'member'::team_role not null,
    user_id   uuid
        constraint invitations_users_id_fk
            references users
            on delete cascade,
    email     text,
    timestamp timestamptz default now()               not null,
    constraint email_or_existing_user
        check ((((user_id IS NOT NULL))::integer + ((email IS NOT NULL))::integer) = 1)
);

