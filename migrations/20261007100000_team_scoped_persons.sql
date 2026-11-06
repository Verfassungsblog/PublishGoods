-- Persons become team- or project-scoped, and user-owned projects/folders go away
-- (every user gets a personal team instead).
--
-- 1. Persons: owner columns + backfill into the 'Default' team.
--    This runs before personal teams are created so that the by-name 'Default' lookup
--    can't be confused by a user who happens to be called 'Default'.
ALTER TABLE persons
    ADD COLUMN owner_team_id uuid
        CONSTRAINT persons_teams_id_fk
            REFERENCES teams ON UPDATE CASCADE ON DELETE CASCADE,
    ADD COLUMN project_id uuid
        CONSTRAINT persons_projects_id_fk
            REFERENCES projects ON UPDATE CASCADE ON DELETE CASCADE;

DO
$$
    DECLARE
        default_team uuid;
    BEGIN
        IF EXISTS (SELECT 1 FROM persons) THEN
            -- same lookup/creation as users::ensure_default_team
            SELECT id INTO default_team FROM teams WHERE name = 'Default' LIMIT 1;
            IF default_team IS NULL THEN
                INSERT INTO teams (name) VALUES ('Default') RETURNING id INTO default_team;
            END IF;
            UPDATE persons SET owner_team_id = default_team;
        END IF;
    END
$$;

ALTER TABLE persons
    ADD CONSTRAINT person_scope
        CHECK (((owner_team_id IS NOT NULL)::integer + (project_id IS NOT NULL)::integer) = 1);

create index persons_owner_team_id_idx on persons (owner_team_id);
create index persons_project_id_idx on persons (project_id);

-- 2. A personal team for every existing user (an ordinary team that is only distinguished
--    by being created together with the account).
CREATE TEMP TABLE personal_teams
(
    user_id uuid PRIMARY KEY,
    team_id uuid NOT NULL DEFAULT gen_random_uuid()
);

INSERT INTO personal_teams (user_id)
SELECT id
FROM users;

-- 'Default' is reserved for the shared default team (it is looked up by name)
INSERT INTO teams (id, name)
SELECT pt.team_id,
       CASE WHEN u.name = 'Default' THEN 'Default (personal)' ELSE u.name END
FROM personal_teams pt
         JOIN users u ON u.id = pt.user_id;

INSERT INTO users_teams (user_id, team_id, role)
SELECT user_id, team_id, 'owner'
FROM personal_teams;

-- 3. Re-home user-owned projects/folders (unused by the app so far) to their owner's team,
--    then drop user ownership entirely.
UPDATE projects p
SET owner_team_id = pt.team_id,
    owner_user_id = NULL
FROM personal_teams pt
WHERE p.owner_user_id = pt.user_id;

UPDATE project_folders f
SET owner_team_id = pt.team_id,
    owner_user_id = NULL
FROM personal_teams pt
WHERE f.owner_user_id = pt.user_id;

DROP TABLE personal_teams;

ALTER TABLE projects
    DROP CONSTRAINT owner,
    DROP COLUMN owner_user_id,
    ALTER COLUMN owner_team_id SET NOT NULL;

ALTER TABLE project_folders
    DROP CONSTRAINT owner,
    DROP COLUMN owner_user_id,
    ALTER COLUMN owner_team_id SET NOT NULL;
