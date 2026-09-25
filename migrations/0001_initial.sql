-- IronMaint 0B initial schema.
--
-- Forward-only. This file is applied at daemon startup if not
-- already present; the verifier (cargo xtask verify-migrations)
-- confirms that the committed set matches what an empty SQLite
-- instance would produce.
--
-- PHASE-0B.md §7-9. Tables map to the 0B persistence surface:
--
--   events             → EventStore
--   projections        → ProjectionStore
--   source_candidates  → CandidateStore (SourceCandidate)
--   release_candidates → CandidateStore (ReleaseCandidate)
--   evidence           → EvidenceStore
--   gate_definitions   → GateStore (definition)
--   gate_results       → GateStore (result)
--   obligations        → ObligationStore
--   operations         → OperationStore
--   artifact_metadata  → ArtifactMetadataStore
--   workspace_metadata → WorkspaceMetadataStore
--
-- Foreign keys: ON. Schema version stored on each row for forward
-- compat (see §98: refuse unsupported schema versions).

CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    filename    TEXT    NOT NULL UNIQUE,
    applied_at  TEXT    NOT NULL
);

-- 7. Event Store
CREATE TABLE IF NOT EXISTS events (
    event_id        TEXT    PRIMARY KEY,
    job_id          TEXT    NOT NULL,
    sequence        INTEGER NOT NULL CHECK (sequence > 0),
    schema_version  TEXT    NOT NULL,
    occurred_at     TEXT    NOT NULL,
    event_type      TEXT    NOT NULL,
    payload_json    TEXT    NOT NULL,
    UNIQUE (job_id, sequence),
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_events_job_sequence
    ON events (job_id, sequence);

-- 8. Projection cache
CREATE TABLE IF NOT EXISTS projections (
    job_id                TEXT    PRIMARY KEY,
    package_json          TEXT    NOT NULL,
    initiating_event_id   TEXT    NOT NULL,
    state                 TEXT    NOT NULL,
    active_candidate_id   TEXT,
    version               INTEGER NOT NULL CHECK (version >= 0),
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL
);

-- Candidates (split: source vs release; same table is also possible
-- but the discriminator makes adapter introspection trivial).
CREATE TABLE IF NOT EXISTS source_candidates (
    candidate_id  TEXT    PRIMARY KEY,
    job_id        TEXT    NOT NULL,
    fingerprint   TEXT    NOT NULL UNIQUE,
    package_json  TEXT    NOT NULL,
    payload_json  TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_source_candidates_job
    ON source_candidates (job_id);

CREATE TABLE IF NOT EXISTS release_candidates (
    candidate_id  TEXT    PRIMARY KEY,
    job_id        TEXT    NOT NULL,
    fingerprint   TEXT    NOT NULL,
    payload_json  TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_release_candidates_job
    ON release_candidates (job_id);

CREATE TABLE IF NOT EXISTS active_source_candidates (
    job_id        TEXT    PRIMARY KEY,
    candidate_id  TEXT    NOT NULL,
    FOREIGN KEY (job_id)        REFERENCES projections (job_id),
    FOREIGN KEY (candidate_id)  REFERENCES source_candidates (candidate_id)
);

-- Evidence
CREATE TABLE IF NOT EXISTS evidence (
    evidence_id  TEXT    PRIMARY KEY,
    job_id       TEXT    NOT NULL,
    candidate    TEXT    NOT NULL,
    schema_version TEXT  NOT NULL,
    payload_json TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_evidence_job
    ON evidence (job_id);
CREATE INDEX IF NOT EXISTS idx_evidence_candidate
    ON evidence (candidate);

-- Gates
CREATE TABLE IF NOT EXISTS gate_definitions (
    gate_id        TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    candidate      TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_gate_definitions_job
    ON gate_definitions (job_id);

CREATE TABLE IF NOT EXISTS gate_results (
    gate_id        TEXT    NOT NULL,
    candidate      TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    PRIMARY KEY (gate_id, candidate),
    FOREIGN KEY (gate_id) REFERENCES gate_definitions (gate_id)
        ON DELETE CASCADE
);

-- Obligations
CREATE TABLE IF NOT EXISTS obligations (
    obligation_id  TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    candidate      TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_obligations_job
    ON obligations (job_id);

-- Privileged operations
CREATE TABLE IF NOT EXISTS operations (
    operation_id   TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_operations_job
    ON operations (job_id);

-- Artifact metadata (blobs live in ironmaint-artifacts, not here)
CREATE TABLE IF NOT EXISTS artifact_metadata (
    artifact_id    TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_artifact_metadata_job
    ON artifact_metadata (job_id);

-- Workspace metadata
CREATE TABLE IF NOT EXISTS workspace_metadata (
    handle         TEXT    PRIMARY KEY,
    job_id         TEXT    NOT NULL,
    revision       INTEGER NOT NULL CHECK (revision >= 0),
    schema_version TEXT    NOT NULL,
    payload_json   TEXT    NOT NULL,
    FOREIGN KEY (job_id) REFERENCES projections (job_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_workspace_metadata_job
    ON workspace_metadata (job_id);
