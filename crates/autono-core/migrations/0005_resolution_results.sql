-- Preserve the original accepted response, rather than reconstructing it from a
-- later run revision. Legacy receipts have no historical response snapshot.
ALTER TABLE engineering_resolution_requests ADD COLUMN result TEXT
    CHECK(result IS NULL OR json_valid(result));
