-- Mnemo schema. Idempotent: executed on every server start.

CREATE TABLE IF NOT EXISTS notes (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  path        text NOT NULL CHECK (path <> '' AND path !~ '(^/|/$|//)'),
  content     text NOT NULL DEFAULT '',
  version     integer NOT NULL DEFAULT 1,
  updated_by  text,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now(),
  search      tsvector GENERATED ALWAYS AS (
                setweight(to_tsvector('simple', path), 'A') ||
                setweight(to_tsvector('simple', content), 'B')
              ) STORED
);
CREATE UNIQUE INDEX IF NOT EXISTS notes_path_uniq ON notes (lower(path));
CREATE INDEX IF NOT EXISTS notes_search_idx ON notes USING gin (search);

-- Empty folders (folders that contain notes are derived from note paths)
CREATE TABLE IF NOT EXISTS folders (
  path text PRIMARY KEY CHECK (path <> '' AND path !~ '(^/|/$|//)')
);

-- Outgoing [[wikilinks]], maintained by trigger so even direct SQL edits stay consistent
CREATE TABLE IF NOT EXISTS links (
  source_id uuid NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  target    text NOT NULL,
  PRIMARY KEY (source_id, target)
);
CREATE INDEX IF NOT EXISTS links_target_idx ON links (lower(target));

CREATE TABLE IF NOT EXISTS plugin_state (
  id       text PRIMARY KEY,
  enabled  boolean NOT NULL DEFAULT false,
  settings jsonb NOT NULL DEFAULT '{}'::jsonb
);

-- ---------------------------------------------------------------------------
-- BEFORE trigger: bump version, timestamps, and record which client wrote.
-- The app sets `app.client_id` per transaction; direct SQL edits get NULL,
-- so every connected client refreshes.
CREATE OR REPLACE FUNCTION notes_before_write() RETURNS trigger AS $$
BEGIN
  NEW.updated_by := nullif(current_setting('app.client_id', true), '');
  IF TG_OP = 'UPDATE' THEN
    IF NEW.content IS NOT DISTINCT FROM OLD.content AND NEW.path IS NOT DISTINCT FROM OLD.path THEN
      RETURN NULL; -- no-op update: skip entirely (no version bump, no notify)
    END IF;
    NEW.version    := OLD.version + 1;
    NEW.created_at := OLD.created_at;
    NEW.updated_at := now();
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS notes_before_write ON notes;
CREATE TRIGGER notes_before_write BEFORE INSERT OR UPDATE ON notes
  FOR EACH ROW EXECUTE FUNCTION notes_before_write();

-- AFTER trigger: refresh links + broadcast through NOTIFY
CREATE OR REPLACE FUNCTION notes_after_write() RETURNS trigger AS $$
DECLARE
  payload json;
BEGIN
  IF TG_OP = 'DELETE' THEN
    payload := json_build_object('kind', 'note', 'op', 'delete', 'id', OLD.id, 'path', OLD.path,
                                 'updatedBy', nullif(current_setting('app.client_id', true), ''));
  ELSE
    IF TG_OP = 'INSERT' OR NEW.content IS DISTINCT FROM OLD.content THEN
      DELETE FROM links WHERE source_id = NEW.id;
      INSERT INTO links (source_id, target)
        SELECT DISTINCT NEW.id, t
        FROM (
          SELECT btrim(split_part(split_part(m[1], '|', 1), '#', 1)) AS t
          FROM regexp_matches(NEW.content, '\[\[([^\]\n]+)\]\]', 'g') AS m
        ) s
        WHERE t <> '';
    END IF;
    payload := json_build_object(
      'kind', 'note', 'op', lower(TG_OP), 'id', NEW.id, 'path', NEW.path,
      'oldPath', CASE WHEN TG_OP = 'UPDATE' AND OLD.path <> NEW.path THEN OLD.path END,
      'version', NEW.version, 'updatedBy', NEW.updated_by, 'updatedAt', NEW.updated_at);
  END IF;
  PERFORM pg_notify('vault_events', payload::text);
  RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS notes_after_write ON notes;
CREATE TRIGGER notes_after_write AFTER INSERT OR UPDATE OR DELETE ON notes
  FOR EACH ROW EXECUTE FUNCTION notes_after_write();

-- Folders and plugin state also broadcast
CREATE OR REPLACE FUNCTION generic_notify() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('vault_events', json_build_object(
    'kind', TG_ARGV[0], 'op', lower(TG_OP),
    'id', CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END -> TG_ARGV[1],
    'updatedBy', nullif(current_setting('app.client_id', true), ''))::text);
  RETURN NULL;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS folders_notify ON folders;
CREATE TRIGGER folders_notify AFTER INSERT OR UPDATE OR DELETE ON folders
  FOR EACH ROW EXECUTE FUNCTION generic_notify('folder', 'path');

-- Icônes de l'explorateur (dossier ou note), nom d'icône Lucide ou emoji. Clé : chemin en minuscules.
CREATE TABLE IF NOT EXISTS path_icons (
  path text PRIMARY KEY,
  icon text NOT NULL CHECK (icon <> '')
);
DROP TRIGGER IF EXISTS path_icons_notify ON path_icons;
CREATE TRIGGER path_icons_notify AFTER INSERT OR UPDATE OR DELETE ON path_icons
  FOR EACH ROW EXECUTE FUNCTION generic_notify('icon', 'path');

DROP TRIGGER IF EXISTS plugin_state_notify ON plugin_state;
CREATE TRIGGER plugin_state_notify AFTER INSERT OR UPDATE OR DELETE ON plugin_state
  FOR EACH ROW EXECUTE FUNCTION generic_notify('plugin', 'id');

-- ===========================================================================
-- Dashboard: projects, tickets, reports (notes linked to tickets)
CREATE TABLE IF NOT EXISTS projects (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name        text NOT NULL CHECK (btrim(name) <> ''),
  description text NOT NULL DEFAULT '',
  github_repo text,                       -- "owner/name"
  position    double precision NOT NULL DEFAULT 0,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS projects_name_uniq ON projects (lower(name));

CREATE TABLE IF NOT EXISTS tickets (
  id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id       uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  number           integer NOT NULL DEFAULT 0,
  title            text NOT NULL CHECK (btrim(title) <> ''),
  body             text NOT NULL DEFAULT '',
  status           text NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'doing', 'blocked', 'review', 'done')),
  tags             text[] NOT NULL DEFAULT '{}',
  priority         smallint NOT NULL DEFAULT 1 CHECK (priority BETWEEN 0 AND 3),
  position         double precision NOT NULL DEFAULT 0,
  github_number    integer,
  github_url       text,
  github_state     text,
  github_synced_at timestamptz,
  created_at       timestamptz NOT NULL DEFAULT now(),
  updated_at       timestamptz NOT NULL DEFAULT now(),
  done_at          timestamptz
);
CREATE UNIQUE INDEX IF NOT EXISTS tickets_number_uniq ON tickets (project_id, number);
CREATE UNIQUE INDEX IF NOT EXISTS tickets_github_uniq ON tickets (project_id, github_number) WHERE github_number IS NOT NULL;
-- Ajouts ultérieurs (idempotents) : ticket qui suit une pull request, labels du dépôt (nom + couleur)
ALTER TABLE tickets ADD COLUMN IF NOT EXISTS github_pr boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS github_labels jsonb NOT NULL DEFAULT '[]'::jsonb;

CREATE TABLE IF NOT EXISTS ticket_reports (
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  note_id   uuid NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (ticket_id, note_id)
);

-- Ticket numbers per project (#1, #2…), timestamps and completion date
CREATE OR REPLACE FUNCTION tickets_before_write() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'INSERT' AND (NEW.number IS NULL OR NEW.number = 0) THEN
    PERFORM pg_advisory_xact_lock(hashtext(NEW.project_id::text));
    SELECT COALESCE(max(number), 0) + 1 INTO NEW.number FROM tickets WHERE project_id = NEW.project_id;
  END IF;
  IF TG_OP = 'UPDATE' THEN
    NEW.updated_at := now();
  END IF;
  IF NEW.status = 'done' AND (TG_OP = 'INSERT' OR OLD.status <> 'done') THEN
    NEW.done_at := now();
  ELSIF NEW.status <> 'done' THEN
    NEW.done_at := NULL;
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS tickets_before_write ON tickets;
CREATE TRIGGER tickets_before_write BEFORE INSERT OR UPDATE ON tickets
  FOR EACH ROW EXECUTE FUNCTION tickets_before_write();

CREATE OR REPLACE FUNCTION projects_touch() RETURNS trigger AS $$
BEGIN NEW.updated_at := now(); RETURN NEW; END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS projects_touch ON projects;
CREATE TRIGGER projects_touch BEFORE UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION projects_touch();

-- Live refresh for the dashboard
DROP TRIGGER IF EXISTS projects_notify ON projects;
CREATE TRIGGER projects_notify AFTER INSERT OR UPDATE OR DELETE ON projects
  FOR EACH ROW EXECUTE FUNCTION generic_notify('board', 'id');
DROP TRIGGER IF EXISTS tickets_notify ON tickets;
CREATE TRIGGER tickets_notify AFTER INSERT OR UPDATE OR DELETE ON tickets
  FOR EACH ROW EXECUTE FUNCTION generic_notify('board', 'id');
DROP TRIGGER IF EXISTS ticket_reports_notify ON ticket_reports;
CREATE TRIGGER ticket_reports_notify AFTER INSERT OR UPDATE OR DELETE ON ticket_reports
  FOR EACH ROW EXECUTE FUNCTION generic_notify('board', 'ticket_id');
