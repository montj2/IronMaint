-- IronMaint 0B.5: durable `CheckDefinition` rows
-- (PHASE-0B.md §44).
--
-- `CheckDefinition` is the runtime's materialisation of an
-- adapter's `PlannedCheck`. Each row binds a
-- `(job_id, candidate, gate_id, tool_capability)` tuple to a
-- stable `CheckId` so `RunCheck { check_id }` and evidence
-- records can reference it.
--
-- Forward-only: this is a new table in the same migration set,
-- no prior table changes. The verifier (`cargo xtask
-- verify-migrations`) treats this file as additive over 0001.

CREATE TABLE IF NOT EXISTS check_definitions (
    check_id       TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    candidate      TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_check_definitions_job
    ON check_definitions (job_id);
