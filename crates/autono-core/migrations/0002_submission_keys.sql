ALTER TABLE runs ADD COLUMN submission_key TEXT;
CREATE UNIQUE INDEX runs_submission_key ON runs(submission_key);
