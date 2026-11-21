-- Templates become team-owned: a template belongs to exactly one team, and only that team's
-- members may see or edit it; projects may only use templates of their own team.
ALTER TABLE project_templates
    ADD COLUMN owner_team_id uuid
        CONSTRAINT project_templates_teams_id_fk
            REFERENCES teams ON UPDATE CASCADE ON DELETE CASCADE;

-- Backfill: a template used by the projects of exactly one team goes to that team, every other
-- template (unused, or shared between teams) goes to the 'Default' team. Projects of other teams
-- keep using a shared template; the ownership rule is enforced whenever a template is chosen.
UPDATE project_templates t
SET owner_team_id = u.team_id
FROM (SELECT template_id, MIN(owner_team_id::text)::uuid AS team_id
      FROM projects
      WHERE template_id IS NOT NULL
      GROUP BY template_id
      HAVING COUNT(DISTINCT owner_team_id) = 1) u
WHERE t.id = u.template_id;

DO
$$
    DECLARE
        default_team uuid;
    BEGIN
        IF EXISTS (SELECT 1 FROM project_templates WHERE owner_team_id IS NULL) THEN
            -- same lookup/creation as users::ensure_default_team
            SELECT id INTO default_team FROM teams WHERE name = 'Default' LIMIT 1;
            IF default_team IS NULL THEN
                INSERT INTO teams (name) VALUES ('Default') RETURNING id INTO default_team;
            END IF;
            UPDATE project_templates SET owner_team_id = default_team WHERE owner_team_id IS NULL;
        END IF;
    END
$$;

ALTER TABLE project_templates
    ALTER COLUMN owner_team_id SET NOT NULL;

create index project_templates_owner_team_id_idx on project_templates (owner_team_id);
