CREATE TABLE engineering_runs (
    id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    status TEXT NOT NULL CHECK(status IN ('PLANNING','AWAITING_APPROVAL','RUNNING','VERIFIED','INTEGRATING','COMPLETED','BLOCKED','FAILED','CANCELLED','INTERRUPTED')),
    document TEXT NOT NULL CHECK(json_valid(document)),
    CHECK(json_extract(document, '$.id') = id),
    CHECK(json_extract(document, '$.revision') = revision),
    CHECK(json_extract(document, '$.status') = status)
);
CREATE TABLE engineering_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES engineering_runs(id),
    timestamp TEXT NOT NULL,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX engineering_events_run_sequence ON engineering_events(run_id, sequence);
CREATE TRIGGER engineering_events_no_update BEFORE UPDATE ON engineering_events
BEGIN SELECT RAISE(ABORT, 'engineering events are append-only'); END;
CREATE TRIGGER engineering_events_no_delete BEFORE DELETE ON engineering_events
BEGIN SELECT RAISE(ABORT, 'engineering events are append-only'); END;
CREATE TABLE engineering_workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES engineering_runs(id),
    path TEXT UNIQUE NOT NULL,
    branch TEXT NOT NULL,
    repository TEXT NOT NULL,
    UNIQUE(repository, branch)
);
