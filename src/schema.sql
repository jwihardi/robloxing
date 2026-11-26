PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS projects(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS milestones(id INTEGER PRIMARY KEY, project INTEGER NOT NULL REFERENCES projects, name TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tasks(id INTEGER PRIMARY KEY, milestone INTEGER NOT NULL REFERENCES milestones, prompt TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS task_attachments(task INTEGER NOT NULL REFERENCES tasks, path TEXT NOT NULL, PRIMARY KEY(task,path));
CREATE TABLE IF NOT EXISTS attempts(
    id INTEGER PRIMARY KEY, task INTEGER NOT NULL REFERENCES tasks, state TEXT NOT NULL, action TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'codex', head TEXT NOT NULL, base TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL, session TEXT NOT NULL DEFAULT '', patch TEXT NOT NULL DEFAULT '', candidate TEXT NOT NULL DEFAULT '', effort TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, attempt INTEGER NOT NULL REFERENCES attempts,
    recorded TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, kind TEXT NOT NULL, summary TEXT);
CREATE TABLE IF NOT EXISTS event_images(event INTEGER PRIMARY KEY REFERENCES events(id), path TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS event_files(event INTEGER NOT NULL REFERENCES events(id), path TEXT NOT NULL, PRIMARY KEY(event,path));
CREATE TABLE IF NOT EXISTS pending_worker_directions(event INTEGER PRIMARY KEY REFERENCES events(id), worker INTEGER NOT NULL REFERENCES workers(id));
CREATE TABLE IF NOT EXISTS annotation_sources(attachment TEXT PRIMARY KEY, original TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS chat_image_events ON events(attempt,id) WHERE kind='workshop/image';
CREATE TABLE IF NOT EXISTS decisions(attempt INTEGER PRIMARY KEY REFERENCES attempts, decision TEXT NOT NULL,
    recorded TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
CREATE TABLE IF NOT EXISTS artifacts(id INTEGER PRIMARY KEY, attempt INTEGER NOT NULL REFERENCES attempts,
    kind TEXT NOT NULL, path TEXT NOT NULL, UNIQUE(attempt, path));

CREATE TABLE IF NOT EXISTS team_settings(id INTEGER PRIMARY KEY CHECK(id=1), lead_model TEXT NOT NULL,
    worker_model TEXT NOT NULL, parallelism INTEGER NOT NULL CHECK(parallelism IN (1,2)), check_args TEXT NOT NULL,
    auto_delegate INTEGER NOT NULL DEFAULT 1,
    lead_effort TEXT NOT NULL DEFAULT '', worker_effort TEXT NOT NULL DEFAULT '',
    override_worker_model INTEGER NOT NULL DEFAULT 0, override_worker_effort INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO team_settings(id,lead_model,worker_model,parallelism,check_args) VALUES(1,'','',2,'[]');
CREATE TABLE IF NOT EXISTS workers(id INTEGER PRIMARY KEY, lead INTEGER NOT NULL REFERENCES attempts,
    name TEXT NOT NULL, spec TEXT NOT NULL, state TEXT NOT NULL, run INTEGER, tries INTEGER NOT NULL DEFAULT 0,
    detail TEXT NOT NULL DEFAULT '', signature TEXT NOT NULL DEFAULT '', repeats INTEGER NOT NULL DEFAULT 0,
    tokens INTEGER, model TEXT, effort TEXT, UNIQUE(lead,name));
CREATE TABLE IF NOT EXISTS checkpoints(lead INTEGER PRIMARY KEY REFERENCES attempts, decisions TEXT NOT NULL, next_task TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_by_attempt ON events(attempt,id);
CREATE VIEW IF NOT EXISTS delivered_user_inputs AS
    SELECT id,attempt,summary FROM events WHERE kind IN ('user/followup','steer/sent');
CREATE INDEX IF NOT EXISTS attempts_by_task ON attempts(task,id);
CREATE INDEX IF NOT EXISTS blocker_events ON events(attempt,id)
    WHERE kind IN ('workshop/blocker','workshop/blocker_resolved');
CREATE TABLE IF NOT EXISTS codex_asset_sessions(session TEXT PRIMARY KEY,pack_tools INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS usage(attempt INTEGER PRIMARY KEY REFERENCES attempts, tokens INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS queued_messages(
    id INTEGER PRIMARY KEY AUTOINCREMENT, attempt INTEGER NOT NULL REFERENCES attempts,
    body TEXT NOT NULL, files TEXT NOT NULL, sending INTEGER NOT NULL DEFAULT 0,
    steering INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS queue_by_chat ON queued_messages(attempt,id);
CREATE TABLE IF NOT EXISTS context_usage(attempt INTEGER PRIMARY KEY REFERENCES attempts, used INTEGER, capacity INTEGER);
CREATE TABLE IF NOT EXISTS verified_team(lead INTEGER PRIMARY KEY REFERENCES attempts, tree TEXT NOT NULL, check_args TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS chat_worker_policy(attempt INTEGER PRIMARY KEY REFERENCES attempts, policy TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS chat_goals(
    attempt INTEGER PRIMARY KEY REFERENCES attempts,
    objective TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active','verifying','paused','complete')),
    detail TEXT NOT NULL DEFAULT '',
    recoverable INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS studio_jobs(
    id INTEGER PRIMARY KEY, area TEXT NOT NULL, title TEXT NOT NULL, request TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'Running', detail TEXT NOT NULL DEFAULT '',
    created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS studio_jobs_area ON studio_jobs(area,id);
CREATE TABLE IF NOT EXISTS event_assets(
    event INTEGER PRIMARY KEY REFERENCES events(id), job INTEGER NOT NULL UNIQUE REFERENCES studio_jobs(id),
    maker TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS decompile_records(
    job INTEGER PRIMARY KEY REFERENCES studio_jobs(id), input_sha TEXT NOT NULL DEFAULT '',
    filter_key TEXT NOT NULL DEFAULT '', format INTEGER NOT NULL DEFAULT 0,
    report_sha TEXT NOT NULL DEFAULT '', symbols TEXT NOT NULL DEFAULT '',
    reused_from INTEGER REFERENCES studio_jobs(id));
CREATE INDEX IF NOT EXISTS decompile_cache ON decompile_records(input_sha, filter_key, format);
