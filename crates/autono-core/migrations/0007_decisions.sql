-- Immutable decision identities and append-only audit facts. Existing runs need no rewrite.
CREATE TABLE engineering_decisions (
    run_id TEXT NOT NULL REFERENCES engineering_runs(id),
    id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    document TEXT NOT NULL CHECK(json_valid(document)),
    PRIMARY KEY(run_id,id),
    UNIQUE(run_id,ordinal)
);
CREATE TABLE engineering_decision_events (
    run_id TEXT NOT NULL,
    decision_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    document TEXT NOT NULL CHECK(json_valid(document)),
    PRIMARY KEY(run_id,decision_id,sequence),
    FOREIGN KEY(run_id,decision_id) REFERENCES engineering_decisions(run_id,id)
);
CREATE TRIGGER decisions_no_update BEFORE UPDATE ON engineering_decisions
BEGIN SELECT RAISE(ABORT, 'decisions are immutable'); END;
CREATE TRIGGER decisions_no_delete BEFORE DELETE ON engineering_decisions
BEGIN SELECT RAISE(ABORT, 'decisions are immutable'); END;
CREATE TRIGGER decision_events_no_update BEFORE UPDATE ON engineering_decision_events
BEGIN SELECT RAISE(ABORT, 'decision events are append-only'); END;
CREATE TRIGGER decision_events_no_delete BEFORE DELETE ON engineering_decision_events
BEGIN SELECT RAISE(ABORT, 'decision events are append-only'); END;
