-- Mnemo schema. Idempotent: executed on every server start.
--
-- Rejoué à chaque connexion, pendant que d'autres appareils écrivent : en régime établi, aucune instruction ne
-- doit verrouiller une table (ALTER TABLE, CREATE INDEX et CREATE TRIGGER le font même quand il n'y a rien à
-- faire). D'où les fonctions mnemo_ensure_* ci-dessous, qui n'agissent que si l'objet manque.
-- D'anciennes versions (bureau, Android) utilisent la même base : uniquement des changements compatibles, et
-- toute nouvelle logique passe par des fonctions / triggers aux noms nouveaux (une ancienne version rejoue son
-- propre schéma et remet ses définitions de notes_before_write, tickets_before_write…).

CREATE OR REPLACE FUNCTION mnemo_ensure_trigger(tbl text, trg text, def text) RETURNS void AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgrelid = tbl::regclass AND tgname = trg AND NOT tgisinternal) THEN
    EXECUTE format('CREATE TRIGGER %I %s', trg, def);
  END IF;
END $$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION mnemo_ensure_index(idx text, def text) RETURNS void AS $$
BEGIN
  IF to_regclass(quote_ident(idx)) IS NULL THEN
    EXECUTE def;
  END IF;
END $$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION mnemo_ensure_column(tbl text, col text, def text) RETURNS void AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_attribute
                 WHERE attrelid = tbl::regclass AND attname = col AND NOT attisdropped) THEN
    EXECUTE format('ALTER TABLE %I ADD COLUMN %I %s', tbl, col, def);
  END IF;
END $$ LANGUAGE plpgsql;

-- Contrainte CHECK ajoutée NOT VALID : les lignes existantes ne font pas échouer la migration, mais toute
-- écriture est vérifiée.
CREATE OR REPLACE FUNCTION mnemo_ensure_check(tbl text, con text, expr text) RETURNS void AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = tbl::regclass AND conname = con) THEN
    EXECUTE format('ALTER TABLE %I ADD CONSTRAINT %I CHECK (%s) NOT VALID', tbl, con, expr);
  END IF;
END $$ LANGUAGE plpgsql;

-- Chemin sûr : aucun segment « . » ou « .. », ni barre oblique inverse, ni caractère de contrôle.
CREATE OR REPLACE FUNCTION mnemo_path_ok(path text) RETURNS boolean AS $$
  SELECT path !~ '(^|/)\.\.?(/|$)' AND position(chr(92) IN path) = 0 AND path !~ '[\x01-\x1f\x7f]'
$$ LANGUAGE sql IMMUTABLE;

CREATE TABLE IF NOT EXISTS notes (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  path        text NOT NULL CHECK (path <> '' AND path !~ '(^/|/$|//)'),
  content     text NOT NULL DEFAULT '',
  version     integer NOT NULL DEFAULT 1,
  updated_by  text,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now(),
  search      tsvector  -- obsolète, toujours NULL (voir ci-dessous)
);
SELECT mnemo_ensure_index('notes_path_uniq', 'CREATE UNIQUE INDEX notes_path_uniq ON notes (lower(path))');
SELECT mnemo_ensure_check('notes', 'notes_path_safe', 'mnemo_path_ok(path)');

-- `search` était une colonne générée (tsvector du contenu) : plus aucune requête ne la lit (la recherche est
-- locale) et sa limite de 1 Mo faisait échouer l'enregistrement des grandes notes, donc toute la synchro.
-- Elle devient une colonne ordinaire, vide : les anciennes versions, qui rejouent
-- `CREATE INDEX IF NOT EXISTS notes_search_idx … (search)` à la connexion, continuent de fonctionner.
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_attribute
             WHERE attrelid = 'notes'::regclass AND attname = 'search' AND NOT attisdropped AND attgenerated <> '') THEN
    ALTER TABLE notes DROP COLUMN search;
    ALTER TABLE notes ADD COLUMN search tsvector;
  END IF;
END $$;

-- Empty folders (folders that contain notes are derived from note paths)
CREATE TABLE IF NOT EXISTS folders (
  path text PRIMARY KEY CHECK (path <> '' AND path !~ '(^/|/$|//)')
);
SELECT mnemo_ensure_check('folders', 'folders_path_safe', 'mnemo_path_ok(path)');

-- Outgoing [[wikilinks]], maintained by trigger so even direct SQL edits stay consistent
CREATE TABLE IF NOT EXISTS links (
  source_id uuid NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  target    text NOT NULL,
  PRIMARY KEY (source_id, target)
);
SELECT mnemo_ensure_index('links_target_idx', 'CREATE INDEX links_target_idx ON links (lower(target))');

-- Une cible démesurée (« [[ » suivi de plusieurs Ko sans retour à la ligne) dépasse la taille d'une entrée
-- d'index et ferait échouer l'enregistrement de la note : elle n'est pas indexée. Trigger sur `links` (et non
-- dans notes_after_write) pour rester en place quand une ancienne version rejoue son schéma.
CREATE OR REPLACE FUNCTION links_skip_long() RETURNS trigger AS $$
BEGIN
  IF length(NEW.target) > 500 THEN
    RETURN NULL;
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;
SELECT mnemo_ensure_trigger('links', 'links_skip_long',
  'BEFORE INSERT OR UPDATE ON links FOR EACH ROW EXECUTE FUNCTION links_skip_long()');

-- Identité de la base : chaque appareil note avec quelle base son dossier a été synchronisé (.mnemo/sync.json)
CREATE TABLE IF NOT EXISTS mnemo_meta (
  key   text PRIMARY KEY,
  value text NOT NULL
);
INSERT INTO mnemo_meta (key, value) VALUES ('db_id', gen_random_uuid()::text) ON CONFLICT (key) DO NOTHING;

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

SELECT mnemo_ensure_trigger('notes', 'notes_before_write',
  'BEFORE INSERT OR UPDATE ON notes FOR EACH ROW EXECUTE FUNCTION notes_before_write()');

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

SELECT mnemo_ensure_trigger('notes', 'notes_after_write',
  'AFTER INSERT OR UPDATE OR DELETE ON notes FOR EACH ROW EXECUTE FUNCTION notes_after_write()');

-- Folders and plugin state also broadcast
CREATE OR REPLACE FUNCTION generic_notify() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('vault_events', json_build_object(
    'kind', TG_ARGV[0], 'op', lower(TG_OP),
    'id', CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END -> TG_ARGV[1],
    'updatedBy', nullif(current_setting('app.client_id', true), ''))::text);
  RETURN NULL;
END $$ LANGUAGE plpgsql;

SELECT mnemo_ensure_trigger('folders', 'folders_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON folders FOR EACH ROW EXECUTE FUNCTION generic_notify(''folder'', ''path'')');

-- Icônes de l'explorateur (dossier ou note), nom d'icône Lucide ou emoji. Clé : chemin en minuscules.
CREATE TABLE IF NOT EXISTS path_icons (
  path text PRIMARY KEY,
  icon text NOT NULL CHECK (icon <> '')
);
SELECT mnemo_ensure_trigger('path_icons', 'path_icons_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON path_icons FOR EACH ROW EXECUTE FUNCTION generic_notify(''icon'', ''path'')');

-- Pièces jointes du dossier des notes (images, PDF, whiteboards…), synchronisées entre appareils.
-- hash : FNV-1a 64 bits du contenu en hexadécimal (calculé de la même façon par l'application mobile).
CREATE TABLE IF NOT EXISTS files (
  path       text PRIMARY KEY CHECK (path <> '' AND path !~ '(^/|/$|//)'),
  data       bytea NOT NULL,
  hash       text NOT NULL,
  size       bigint NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
SELECT mnemo_ensure_check('files', 'files_path_safe', 'mnemo_path_ok(path)');
SELECT mnemo_ensure_trigger('files', 'files_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON files FOR EACH ROW EXECUTE FUNCTION generic_notify(''file'', ''path'')');

SELECT mnemo_ensure_trigger('plugin_state', 'plugin_state_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON plugin_state FOR EACH ROW EXECUTE FUNCTION generic_notify(''plugin'', ''id'')');

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
SELECT mnemo_ensure_index('projects_name_uniq', 'CREATE UNIQUE INDEX projects_name_uniq ON projects (lower(name))');

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
SELECT mnemo_ensure_index('tickets_number_uniq', 'CREATE UNIQUE INDEX tickets_number_uniq ON tickets (project_id, number)');
SELECT mnemo_ensure_index('tickets_github_uniq',
  'CREATE UNIQUE INDEX tickets_github_uniq ON tickets (project_id, github_number) WHERE github_number IS NOT NULL');
-- Ajouts ultérieurs (idempotents) : ticket qui suit une pull request, labels du dépôt (nom + couleur)
SELECT mnemo_ensure_column('tickets', 'github_pr', 'boolean NOT NULL DEFAULT false');
SELECT mnemo_ensure_column('projects', 'github_labels', $$jsonb NOT NULL DEFAULT '[]'::jsonb$$);
-- Synchronisation GitHub champ par champ :
--   content_changed_at : dernière modification d'un champ que GitHub voit (titre, description, tags, ouvert / terminé) ;
--                        NULL sur les lignes antérieures (updated_at fait alors foi) ;
--   github_base        : l'issue telle que vue à la dernière synchro {title, body, labels, open, updated_at}.
SELECT mnemo_ensure_column('tickets', 'content_changed_at', 'timestamptz');
SELECT mnemo_ensure_column('tickets', 'github_base', 'jsonb');

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

SELECT mnemo_ensure_trigger('tickets', 'tickets_before_write',
  'BEFORE INSERT OR UPDATE ON tickets FOR EACH ROW EXECUTE FUNCTION tickets_before_write()');

-- Lien GitHub et numéro d'un ticket modifié. Trigger distinct de tickets_before_write (voir l'en-tête).
--   * changement de projet : nouveau numéro dans le projet d'arrivée (l'ancien y est sans doute déjà pris) ; si le
--     dépôt n'est pas le même, ou si l'issue y a déjà son ticket, le lien GitHub est effacé (ticket « non publié ») :
--     sinon la synchro suivante écraserait l'issue de même numéro d'un autre dépôt ;
--   * content_changed_at ne bouge que si un champ visible sur GitHub change (pas la priorité ni la position).
CREATE OR REPLACE FUNCTION tickets_track() RETURNS trigger AS $$
DECLARE
  old_repo text;
  new_repo text;
BEGIN
  IF TG_OP = 'INSERT' THEN
    NEW.content_changed_at := COALESCE(NEW.content_changed_at, now());
    RETURN NEW;
  END IF;
  IF NEW.project_id IS DISTINCT FROM OLD.project_id THEN
    PERFORM pg_advisory_xact_lock(hashtext(NEW.project_id::text));
    SELECT COALESCE(max(number), 0) + 1 INTO NEW.number FROM tickets WHERE project_id = NEW.project_id;
    IF NEW.github_number IS NOT NULL THEN
      SELECT lower(github_repo) INTO old_repo FROM projects WHERE id = OLD.project_id;
      SELECT lower(github_repo) INTO new_repo FROM projects WHERE id = NEW.project_id;
      IF old_repo IS DISTINCT FROM new_repo OR EXISTS (
           SELECT 1 FROM tickets WHERE project_id = NEW.project_id AND github_number = NEW.github_number) THEN
        NEW.github_number := NULL;
        NEW.github_url := NULL;
        NEW.github_state := NULL;
        NEW.github_synced_at := NULL;
        NEW.github_base := NULL;
        NEW.github_pr := false;
      END IF;
    END IF;
  END IF;
  IF NEW.title IS DISTINCT FROM OLD.title OR NEW.body IS DISTINCT FROM OLD.body OR NEW.tags IS DISTINCT FROM OLD.tags
     OR (NEW.status = 'done') IS DISTINCT FROM (OLD.status = 'done') THEN
    NEW.content_changed_at := now();
  ELSE
    NEW.content_changed_at := COALESCE(OLD.content_changed_at, OLD.updated_at);
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;
SELECT mnemo_ensure_trigger('tickets', 'tickets_track',
  'BEFORE INSERT OR UPDATE ON tickets FOR EACH ROW EXECUTE FUNCTION tickets_track()');

-- Le dépôt d'un projet change (ou est retiré) : ses tickets ne désignent plus les issues de l'ancien dépôt.
CREATE OR REPLACE FUNCTION projects_repo_changed() RETURNS trigger AS $$
BEGIN
  IF lower(NEW.github_repo) IS DISTINCT FROM lower(OLD.github_repo) THEN
    UPDATE tickets
       SET github_number = NULL, github_url = NULL, github_state = NULL, github_synced_at = NULL,
           github_base = NULL, github_pr = false
     WHERE project_id = NEW.id AND github_number IS NOT NULL;
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
SELECT mnemo_ensure_trigger('projects', 'projects_repo_changed',
  'AFTER UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION projects_repo_changed()');

CREATE OR REPLACE FUNCTION projects_touch() RETURNS trigger AS $$
BEGIN NEW.updated_at := now(); RETURN NEW; END $$ LANGUAGE plpgsql;
SELECT mnemo_ensure_trigger('projects', 'projects_touch',
  'BEFORE UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION projects_touch()');

-- Live refresh for the dashboard
SELECT mnemo_ensure_trigger('projects', 'projects_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON projects FOR EACH ROW EXECUTE FUNCTION generic_notify(''board'', ''id'')');
SELECT mnemo_ensure_trigger('tickets', 'tickets_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON tickets FOR EACH ROW EXECUTE FUNCTION generic_notify(''board'', ''id'')');
SELECT mnemo_ensure_trigger('ticket_reports', 'ticket_reports_notify',
  'AFTER INSERT OR UPDATE OR DELETE ON ticket_reports FOR EACH ROW EXECUTE FUNCTION generic_notify(''board'', ''ticket_id'')');
