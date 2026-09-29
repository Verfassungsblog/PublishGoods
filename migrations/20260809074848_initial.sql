-- Add migration script here
create type team_role as enum ('owner', 'admin', 'member');

create table users
(
    id            uuid default gen_random_uuid() not null
        constraint users_pk
            primary key,
    email         text                           not null
        constraint users_pk_2
            unique,
    name          text                           not null,
    password_hash text,
    locked_until  timestamp with time zone
);

create table teams
(
    id   uuid default gen_random_uuid() not null
        constraint teams_pk
            primary key,
    name text                           not null
);

create table users_teams
(
    user_id uuid                                  not null
        constraint users_teams_users_id_fk
            references users
            on update cascade on delete cascade,
    team_id uuid                                  not null
        constraint users_teams_teams_id_fk
            references teams
            on update cascade on delete cascade,
    role    team_role default 'member'::team_role not null,
    constraint users_teams_pk
        primary key (user_id, team_id)
);

create table user_login_attempts
(
    id        uuid                     default gen_random_uuid() not null
        constraint user_login_attempts_pk
            primary key,
    user_id   uuid                                               not null
        constraint user_login_attempts_users_id_fk
            references users
            on update cascade on delete cascade,
    timestamp timestamp with time zone default now()             not null
);

create table persons
(
    id          uuid default gen_random_uuid() not null
        constraint persons_pk
            primary key,
    first_names text,
    last_names  text                           not null,
    orcid       text,
    gnd         text,
    ror         text
);

create table biographies
(
    person_id uuid not null
        constraint biographies_persons_id_fk
            references persons
            on update cascade on delete cascade,
    content   text not null,
    language  text not null,
    constraint biographies_pk
        primary key (person_id, language)
);

create table project_templates
(
    id          uuid default gen_random_uuid() not null
        constraint project_templates_pk
            primary key,
    version     uuid default gen_random_uuid() not null,
    name        text                           not null,
    description text
);

create table export_formats
(
    id                  uuid default gen_random_uuid() not null
        constraint export_formats_pk
            primary key,
    project_template_id uuid                           not null
        constraint export_formats_project_templates_id_fk
            references project_templates
            on update cascade on delete cascade,
    slug                text                           not null,
    name                text                           not null,
    preview_pdf_path    text,
    output_files        text[],
    export_steps        jsonb
);

create table project_folders
(
    id            uuid default gen_random_uuid() not null
        constraint project_folders_pk
            primary key,
    name          text                           not null,
    owner_team_id uuid
        constraint project_folders_teams_id_fk
            references teams
            on update cascade on delete cascade,
    owner_user_id uuid
        constraint project_folders_users_id_fk
            references users
            on update cascade on delete cascade,
    parent        uuid
        constraint project_folders_project_folders_id_fk
            references project_folders
            on update cascade on delete set null,
    constraint owner
        check ((((owner_user_id IS NOT NULL))::integer + ((owner_team_id IS NOT NULL))::integer) = 1)
);

create table projects
(
    id                   uuid                     default gen_random_uuid() not null
        constraint projects_pk
            primary key,
    description          text,
    template_id          uuid
        constraint projects_project_templates_id_fk
            references project_templates
            on update cascade on delete set null,
    last_interaction     timestamp with time zone default now()             not null,
    title                text                                               not null,
    subtitle             text,
    web_url              text,
    publish_date         date,
    languages            text[],
    number_of_pages      integer,
    short_abstract       text,
    long_abstract        text,
    keywords             jsonb,
    ddc                  text,
    license              text,
    series               text,
    volume               text,
    edition              text,
    publisher            text,
    custom_fields        jsonb,
    toc_enabled          boolean,
    csl_style            text,
    csl_language_code    text,
    cover_image_path     text,
    backcover_image_path text,
    add_soft_hyphens     boolean,
    identifiers          jsonb,
    folder               uuid
        constraint projects_project_folders_id_fk
            references project_folders
            on update cascade on delete set null,
    owner_team_id        uuid
        constraint projects_teams_id_fk
            references teams
            on update cascade on delete cascade,
    owner_user_id        uuid
        constraint projects_users_id_fk
            references users
            on update cascade on delete cascade,
    constraint owner
        check ((((owner_user_id IS NOT NULL))::integer + ((owner_team_id IS NOT NULL))::integer) = 1)
);

create table persons_projects
(
    person_id  uuid
        constraint persons_projects_persons_id_fk
            references persons
            on update cascade on delete cascade,
    name       text,
    project_id uuid                           not null
        constraint persons_projects_projects_id_fk
            references projects
            on update cascade on delete cascade,
    role       text                           not null,
    position   double precision               not null,
    id         uuid default gen_random_uuid() not null
        constraint persons_projects_pk
            primary key,
    constraint person_id_or_name
        check ((((person_id IS NOT NULL))::integer + ((name IS NOT NULL))::integer) = 1)
);

comment on column persons_projects.name is 'alternative if no person id is given';

create table sections
(
    id                          uuid    default gen_random_uuid() not null
        constraint sections_pk
            primary key,
    project_id                  uuid                              not null
        constraint sections_projects_id_fk
            references projects
            on update cascade on delete cascade,
    parent_section              uuid
        constraint sections_sections_id_fk
            references sections
            on update cascade on delete set null,
    position                    double precision                  not null,
    visible_in_toc              boolean default true              not null,
    css_classes                 text[],
    title                       text                              not null,
    toc_title_subtitle_override text,
    subtitle                    text,
    web_url                     text,
    publish_date                date,
    language                    text,
    custom_fields               jsonb,
    identifiers                 jsonb
);

create table persons_sections
(
    person_id  uuid
        constraint persons_sections_persons_id_fk
            references persons
            on update cascade on delete cascade,
    name       text,
    section_id uuid                           not null
        constraint persons_sections_section_id_fk
            references sections
            on update cascade on delete cascade,
    role       text                           not null,
    position   double precision               not null,
    id         uuid default gen_random_uuid() not null
        constraint persons_sections_pk
            primary key,
    constraint person_id_or_name
        check ((((person_id IS NOT NULL))::integer + ((name IS NOT NULL))::integer) = 1)
);

comment on column persons_sections.name is 'alternative if no person id is given';

create table bibliography_folders
(
    id         uuid default gen_random_uuid() not null
        constraint bibliography_folders_pk
            primary key,
    name       text                           not null,
    parent     uuid
        constraint bibliography_folders_bibliography_folders_id_fk
            references bibliography_folders
            on update cascade on delete set null,
    project_id uuid                           not null
        constraint bibliography_folders_projects_id_fk
            references projects
            on update cascade on delete cascade
);

create table bibliography_entries
(
    id         uuid default gen_random_uuid() not null
        constraint bibliography_entries_pk
            primary key,
    data       jsonb,
    folder     uuid
        constraint bibliography_entries_bibliography_folders_id_fk
            references bibliography_folders
            on update cascade on delete set null,
    project_id uuid                           not null
        constraint bibliography_entries_projects_id_fk
            references projects
            on update cascade on delete cascade
);

-- ============================================================================
--  Indexes on foreign-key columns (PKs and unique constraints are already
--  indexed; leading columns of composite PKs are covered too).
-- ============================================================================
create index users_teams_team_id_idx on users_teams (team_id);

create index user_login_attempts_user_id_idx on user_login_attempts (user_id);

create index export_formats_project_template_id_idx on export_formats (project_template_id);

create index project_folders_parent_idx on project_folders (parent);
create index project_folders_owner_team_id_idx on project_folders (owner_team_id);
create index project_folders_owner_user_id_idx on project_folders (owner_user_id);

create index projects_template_id_idx on projects (template_id);
create index projects_folder_idx on projects (folder);
create index projects_owner_team_id_idx on projects (owner_team_id);
create index projects_owner_user_id_idx on projects (owner_user_id);

create index persons_projects_project_id_idx on persons_projects (project_id);
create index persons_projects_person_id_idx on persons_projects (person_id);

create index sections_project_id_idx on sections (project_id);
create index sections_parent_section_idx on sections (parent_section);

create index persons_sections_section_id_idx on persons_sections (section_id);
create index persons_sections_person_id_idx on persons_sections (person_id);

create index bibliography_folders_project_id_idx on bibliography_folders (project_id);
create index bibliography_folders_parent_idx on bibliography_folders (parent);

create index bibliography_entries_project_id_idx on bibliography_entries (project_id);
create index bibliography_entries_folder_idx on bibliography_entries (folder);

