-- Documents remain the checkpoint authority. These inspectable projections share
-- the same revision/event transaction; no existing migration is rewritten.
CREATE TABLE engineering_preflight (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES engineering_runs(id),
    revision INTEGER NOT NULL,
    document TEXT NOT NULL CHECK(json_valid(document))
);
CREATE TABLE engineering_human_actions (
    run_id TEXT NOT NULL REFERENCES engineering_runs(id),
    action_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('PENDING','RESOLVED','DEFERRED','REJECTED')),
    document TEXT NOT NULL CHECK(json_valid(document)),
    PRIMARY KEY(run_id, action_id)
);
CREATE TABLE engineering_resolution_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES engineering_runs(id),
    action_id TEXT NOT NULL,
    document TEXT NOT NULL CHECK(json_valid(document))
);
CREATE TRIGGER engineering_resolution_no_update BEFORE UPDATE ON engineering_resolution_requests
BEGIN SELECT RAISE(ABORT, 'resolution requests are append-only'); END;
CREATE TRIGGER engineering_resolution_no_delete BEFORE DELETE ON engineering_resolution_requests
BEGIN SELECT RAISE(ABORT, 'resolution requests are append-only'); END;
