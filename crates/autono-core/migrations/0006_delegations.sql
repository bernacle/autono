-- Proposals confer no authority. Consent is recorded with initial run creation.
CREATE TABLE engineering_delegation_proposals (
    run_id TEXT PRIMARY KEY NOT NULL,
    document TEXT NOT NULL CHECK(json_valid(document))
);
CREATE TABLE engineering_delegations (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES engineering_runs(id),
    document TEXT NOT NULL CHECK(json_valid(document))
);
CREATE TRIGGER delegation_proposals_no_update BEFORE UPDATE ON engineering_delegation_proposals
BEGIN SELECT RAISE(ABORT, 'delegation proposals are immutable'); END;
CREATE TRIGGER delegation_proposals_no_delete BEFORE DELETE ON engineering_delegation_proposals
BEGIN SELECT RAISE(ABORT, 'delegation proposals are immutable'); END;
CREATE TRIGGER delegations_no_update BEFORE UPDATE ON engineering_delegations
BEGIN SELECT RAISE(ABORT, 'delegations are immutable'); END;
CREATE TRIGGER delegations_no_delete BEFORE DELETE ON engineering_delegations
BEGIN SELECT RAISE(ABORT, 'delegations are immutable'); END;
