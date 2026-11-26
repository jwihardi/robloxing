-- Keep retired IDs reserved. Notifications and screenshot album entries refer
-- to chat IDs, and private workspace directories may still be open on Windows.
CREATE TABLE IF NOT EXISTS retired_workspaces (
    kind TEXT NOT NULL CHECK(kind IN ('chat','worker')),
    id INTEGER NOT NULL CHECK(id>0),
    pending INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(kind,id)
);
CREATE INDEX IF NOT EXISTS pending_workspace_cleanup ON retired_workspaces(kind,id) WHERE pending=1;

CREATE TRIGGER IF NOT EXISTS delete_chat_records BEFORE DELETE ON attempts BEGIN
    SELECT RAISE(ABORT,'Finish or stop the current operation before deleting this chat.')
    WHERE OLD.state IN ('preparing','running','applying') OR EXISTS (
        SELECT 1 FROM workers WHERE lead=OLD.id AND state IN ('running','integrating')
    );
    INSERT INTO retired_workspaces(kind,id) VALUES('chat',OLD.id);
    INSERT INTO retired_workspaces(kind,id) SELECT 'worker',id FROM workers WHERE lead=OLD.id;
    DELETE FROM pending_worker_directions WHERE event IN (SELECT id FROM events WHERE attempt=OLD.id)
        OR worker IN (SELECT id FROM workers WHERE lead=OLD.id);
    DELETE FROM event_assets WHERE event IN (SELECT id FROM events WHERE attempt=OLD.id);
    DELETE FROM event_images WHERE event IN (SELECT id FROM events WHERE attempt=OLD.id);
    DELETE FROM event_files WHERE event IN (SELECT id FROM events WHERE attempt=OLD.id);
    DELETE FROM events WHERE attempt=OLD.id;
    DELETE FROM workers WHERE lead=OLD.id;
    DELETE FROM checkpoints WHERE lead=OLD.id;
    DELETE FROM verified_team WHERE lead=OLD.id;
    DELETE FROM artifacts WHERE attempt=OLD.id;
    DELETE FROM decisions WHERE attempt=OLD.id;
    DELETE FROM usage WHERE attempt=OLD.id;
    DELETE FROM context_usage WHERE attempt=OLD.id;
    DELETE FROM chat_worker_policy WHERE attempt=OLD.id;
    DELETE FROM chat_goals WHERE attempt=OLD.id;
    DELETE FROM queued_messages WHERE attempt=OLD.id;
    DELETE FROM task_heads WHERE attempt=OLD.id;
    DELETE FROM codex_asset_sessions WHERE session=OLD.session AND OLD.provider='codex'
        AND NOT EXISTS (SELECT 1 FROM attempts WHERE id<>OLD.id AND session=OLD.session AND provider='codex');
END;

CREATE TRIGGER IF NOT EXISTS delete_chat_task AFTER DELETE ON attempts BEGIN
    -- Retried chats share their task and initial attachments.
    INSERT OR IGNORE INTO task_heads(task,attempt)
        SELECT task,id FROM attempts WHERE task=OLD.task ORDER BY id DESC LIMIT 1;
    DELETE FROM task_attachments WHERE task=OLD.task AND NOT EXISTS (SELECT 1 FROM attempts WHERE task=OLD.task);
    DELETE FROM tasks WHERE id=OLD.task AND NOT EXISTS (SELECT 1 FROM attempts WHERE task=OLD.task);
END;
