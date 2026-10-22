-- Human Actions remain the blocker identity and current-state projection.
-- Append-only classification/status evidence shares the existing run transaction.
CREATE TABLE engineering_prerequisite_events (
    run_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    document TEXT NOT NULL,
    PRIMARY KEY (run_id, action_id, sequence),
    FOREIGN KEY (run_id, action_id) REFERENCES engineering_human_actions(run_id, action_id)
);
CREATE TRIGGER prerequisite_events_no_update BEFORE UPDATE ON engineering_prerequisite_events
BEGIN SELECT RAISE(ABORT, 'prerequisite evidence is immutable'); END;
CREATE TRIGGER prerequisite_events_no_delete BEFORE DELETE ON engineering_prerequisite_events
BEGIN SELECT RAISE(ABORT, 'prerequisite evidence is immutable'); END;
