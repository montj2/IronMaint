//! In-memory `IronMaintStore` implementation, used by every
//! sub-trait's smoke test and (in later commits) by the runtime
//! integration tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use ironmaint_core::{
    ArtifactId, CandidateFingerprint, CandidateId, EvidenceId, GateId, JobId, JobProjection,
    ObligationId, OperationId, ReleaseCandidateId, SourceCandidate,
};
use ironmaint_evidence::{Evidence, GateDefinition, GateResult};
use ironmaint_policy::{AuthorizationState, Obligation, PrivilegedOperation, ReleaseCandidate};

use crate::artifact::{ArtifactMetadataStore, ArtifactRecord};
use crate::candidate::CandidateStore;
use crate::envelope::EventEnvelope;
use crate::error::StoreError;
use crate::event::EventStore;
use crate::evidence::EvidenceStore;
use crate::gate::GateStore;
use crate::obligation::ObligationStore;
use crate::operation::OperationStore;
use crate::projection::ProjectionStore;
use crate::workspace::{WorkspaceHandle, WorkspaceMetadataStore, WorkspaceState};

/// In-memory store, suitable for tests and for the runtime's
/// "no persistence" mode (which is not a 0B feature but the trait
/// shape supports it).
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
pub struct MockStore {
    inner: Arc<RwLock<MockInner>>,
}

#[derive(Debug, Default, Clone)]
struct MockInner {
    events: BTreeMap<(JobId, u64), EventEnvelope>,
    /// Track max sequence per job for [`EventStore::next_sequence`].
    max_sequence: BTreeMap<JobId, u64>,
    projections: HashMap<JobId, JobProjection>,
    sources: HashMap<CandidateId, SourceCandidate>,
    /// [`CandidateFingerprint`] does not implement `Ord` (PHASE-0A
    /// §13 — fingerprints are opaque hashes), so its secondary
    /// indexes use `HashMap` with linear-scan lookups.
    source_by_fp: HashMap<CandidateFingerprint, CandidateId>,
    active_source: HashMap<JobId, CandidateId>,
    sources_by_job: HashMap<JobId, Vec<CandidateId>>,
    releases: HashMap<ReleaseCandidateId, ReleaseCandidate>,
    evidence: HashMap<EvidenceId, Evidence>,
    evidence_by_job: HashMap<JobId, Vec<EvidenceId>>,
    evidence_by_fp: HashMap<CandidateFingerprint, Vec<EvidenceId>>,
    gate_defs: HashMap<GateId, GateDefinition>,
    /// `(GateId, CandidateFingerprint)` lookup. Keyed by gate id
    /// then scanned for the fingerprint because `CandidateFingerprint`
    /// has no `Ord`.
    gate_results: HashMap<GateId, Vec<(CandidateFingerprint, GateResult)>>,
    gates_by_job: HashMap<JobId, Vec<GateId>>,
    obligations: HashMap<ObligationId, Obligation>,
    obligations_by_job: HashMap<JobId, Vec<ObligationId>>,
    operations: HashMap<OperationId, PrivilegedOperation>,
    operations_executing: Vec<OperationId>,
    operations_by_job: HashMap<JobId, Vec<OperationId>>,
    artifacts: HashMap<ArtifactId, ArtifactRecord>,
    artifacts_by_job: HashMap<JobId, Vec<ArtifactId>>,
    workspaces: HashMap<WorkspaceHandle, WorkspaceState>,
    workspaces_by_job: HashMap<JobId, Vec<WorkspaceHandle>>,
}

impl MockStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, MockInner> {
        self.inner.read().expect("MockStore lock poisoned")
    }

    fn write(&self) -> RwLockWriteGuard<'_, MockInner> {
        self.inner.write().expect("MockStore lock poisoned")
    }

    fn find_gate_result<'a>(
        entries: &'a [(CandidateFingerprint, GateResult)],
        fingerprint: &'a CandidateFingerprint,
    ) -> Option<&'a GateResult> {
        entries
            .iter()
            .find(|(fp, _)| fp == fingerprint)
            .map(|(_, r)| r)
    }
}

impl EventStore for MockStore {
    async fn append_event(&self, event: &EventEnvelope) -> Result<(), StoreError> {
        let mut w = self.write();
        let expected = w.max_sequence.get(&event.job_id).copied().unwrap_or(0) + 1;
        if event.sequence != expected {
            return Err(StoreError::sequence_out_of_range(
                event.job_id,
                expected,
                event.sequence,
            ));
        }
        w.events
            .insert((event.job_id, event.sequence), event.clone());
        w.max_sequence.insert(event.job_id, event.sequence);
        Ok(())
    }

    async fn get_event(&self, job_id: JobId, sequence: u64) -> Result<EventEnvelope, StoreError> {
        self.read()
            .events
            .get(&(job_id, sequence))
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("event ({job_id}, {sequence})")))
    }

    async fn list_events_for_job(
        &self,
        job_id: JobId,
        start: u64,
        end: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let upper = end.unwrap_or(u64::MAX);
        let r = self.read();
        Ok(r.events
            .range((job_id, start)..=(job_id, upper))
            .map(|(_, v)| v.clone())
            .collect())
    }

    async fn next_sequence(&self, job_id: JobId) -> Result<u64, StoreError> {
        Ok(self.read().max_sequence.get(&job_id).copied().unwrap_or(0) + 1)
    }
}

impl ProjectionStore for MockStore {
    async fn get_projection(&self, job_id: JobId) -> Result<JobProjection, StoreError> {
        self.read()
            .projections
            .get(&job_id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("projection for {job_id}")))
    }

    async fn put_projection(
        &self,
        projection: &JobProjection,
        expected_version: u64,
    ) -> Result<(), StoreError> {
        let mut w = self.write();
        let job_id = projection.job.id;
        match w.projections.get(&job_id) {
            Some(existing) if existing.version != expected_version => {
                return Err(StoreError::conflict(format!(
                    "expected_version={expected_version}, found={}",
                    existing.version
                )));
            }
            None if expected_version != 0 => {
                return Err(StoreError::conflict(format!(
                    "first write requires expected_version=0, got {expected_version}"
                )));
            }
            _ => {}
        }
        w.projections.insert(job_id, projection.clone());
        Ok(())
    }

    async fn rebuild_projection(&self, job_id: JobId) -> Result<JobProjection, StoreError> {
        use ironmaint_state::ProjectionApply;
        let events = self.list_events_for_job(job_id, 1, None).await?;
        let Some(first) = events.first() else {
            return Err(StoreError::not_found(format!("no events for {job_id}")));
        };
        let initial = match &first.event {
            ironmaint_state::JobEvent::Transitioned(t) => t.projection_after.clone(),
            ironmaint_state::JobEvent::Domain(_) => {
                return Err(StoreError::corrupt(
                    "first event is a Domain reference; no projection to seed rebuild",
                ));
            }
        };
        let mut proj = initial;
        for env in events.iter().skip(1) {
            proj = ProjectionApply::apply(&proj, &env.event, env.occurred_at);
        }
        Ok(proj)
    }
}

impl CandidateStore for MockStore {
    async fn put_source_candidate(
        &self,
        candidate: &SourceCandidate,
    ) -> Result<CandidateId, StoreError> {
        let id = candidate.id();
        let fp = candidate.fingerprint().clone();
        let job = candidate.job_id();
        let mut w = self.write();
        let already = w.sources.contains_key(&id);
        w.sources.insert(id, candidate.clone());
        w.source_by_fp.insert(fp, id);
        if !already {
            w.sources_by_job.entry(job).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_source_candidate(&self, id: CandidateId) -> Result<SourceCandidate, StoreError> {
        self.read()
            .sources
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("source candidate {id}")))
    }

    async fn put_release_candidate(
        &self,
        candidate: &ReleaseCandidate,
    ) -> Result<ReleaseCandidateId, StoreError> {
        let id = candidate.id;
        self.write().releases.insert(id, candidate.clone());
        Ok(id)
    }

    async fn get_release_candidate(
        &self,
        id: ReleaseCandidateId,
    ) -> Result<ReleaseCandidate, StoreError> {
        self.read()
            .releases
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("release candidate {id}")))
    }

    async fn list_source_candidates_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<CandidateId>, StoreError> {
        Ok(self
            .read()
            .sources_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn active_source_candidate(
        &self,
        job_id: JobId,
    ) -> Result<Option<CandidateId>, StoreError> {
        Ok(self.read().active_source.get(&job_id).copied())
    }

    async fn set_active_source_candidate(
        &self,
        job_id: JobId,
        id: CandidateId,
    ) -> Result<(), StoreError> {
        self.write().active_source.insert(job_id, id);
        Ok(())
    }

    async fn find_source_by_fingerprint(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Option<CandidateId>, StoreError> {
        Ok(self.read().source_by_fp.get(fingerprint).copied())
    }
}

impl EvidenceStore for MockStore {
    async fn put_evidence(
        &self,
        evidence: &Evidence,
        job_id: JobId,
    ) -> Result<EvidenceId, StoreError> {
        let id = evidence.id;
        let fp = evidence.candidate.clone();
        let mut w = self.write();
        let already = w.evidence.contains_key(&id);
        w.evidence.insert(id, evidence.clone());
        if !already {
            w.evidence_by_job.entry(job_id).or_default().push(id);
            w.evidence_by_fp.entry(fp).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_evidence(&self, id: EvidenceId) -> Result<Evidence, StoreError> {
        self.read()
            .evidence
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("evidence {id}")))
    }

    async fn list_evidence_for_job(&self, job_id: JobId) -> Result<Vec<Evidence>, StoreError> {
        let r = self.read();
        let Some(ids) = r.evidence_by_job.get(&job_id) else {
            return Ok(Vec::new());
        };
        Ok(ids
            .iter()
            .filter_map(|id| r.evidence.get(id).cloned())
            .collect())
    }

    async fn list_evidence_for_candidate(
        &self,
        fingerprint: &CandidateFingerprint,
    ) -> Result<Vec<Evidence>, StoreError> {
        let r = self.read();
        let Some(ids) = r.evidence_by_fp.get(fingerprint) else {
            return Ok(Vec::new());
        };
        Ok(ids
            .iter()
            .filter_map(|id| r.evidence.get(id).cloned())
            .collect())
    }
}

impl GateStore for MockStore {
    async fn put_gate_definition(
        &self,
        gate: &GateDefinition,
        job_id: JobId,
    ) -> Result<GateId, StoreError> {
        let id = gate.id;
        let mut w = self.write();
        let already = w.gate_defs.contains_key(&id);
        w.gate_defs.insert(id, gate.clone());
        if !already {
            w.gates_by_job.entry(job_id).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_gate_definition(&self, id: GateId) -> Result<GateDefinition, StoreError> {
        self.read()
            .gate_defs
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("gate definition {id}")))
    }

    async fn put_gate_result(
        &self,
        gate_id: GateId,
        fingerprint: &CandidateFingerprint,
        result: &GateResult,
    ) -> Result<(), StoreError> {
        let mut w = self.write();
        w.gate_results
            .entry(gate_id)
            .or_default()
            .push((fingerprint.clone(), result.clone()));
        Ok(())
    }

    async fn get_gate_result(
        &self,
        gate_id: GateId,
        fingerprint: &CandidateFingerprint,
    ) -> Result<GateResult, StoreError> {
        let r = self.read();
        let Some(entries) = r.gate_results.get(&gate_id) else {
            return Err(StoreError::not_found(format!(
                "gate result ({gate_id}, {fingerprint})"
            )));
        };
        Self::find_gate_result(entries, fingerprint)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("gate result ({gate_id}, {fingerprint})")))
    }

    async fn list_gates_for_job(&self, job_id: JobId) -> Result<Vec<GateId>, StoreError> {
        Ok(self
            .read()
            .gates_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }
}

impl ObligationStore for MockStore {
    async fn put_obligation(
        &self,
        obligation: &Obligation,
        job_id: JobId,
    ) -> Result<ObligationId, StoreError> {
        let id = obligation.id;
        let mut w = self.write();
        let already = w.obligations.contains_key(&id);
        w.obligations.insert(id, obligation.clone());
        if !already {
            w.obligations_by_job.entry(job_id).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_obligation(&self, id: ObligationId) -> Result<Obligation, StoreError> {
        self.read()
            .obligations
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("obligation {id}")))
    }

    async fn update_obligation(
        &self,
        id: ObligationId,
        updated: &Obligation,
    ) -> Result<(), StoreError> {
        let mut w = self.write();
        if !w.obligations.contains_key(&id) {
            return Err(StoreError::not_found(format!("obligation {id}")));
        }
        w.obligations.insert(id, updated.clone());
        Ok(())
    }

    async fn list_obligations_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<ObligationId>, StoreError> {
        Ok(self
            .read()
            .obligations_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }
}

impl OperationStore for MockStore {
    async fn put_operation(
        &self,
        operation: &PrivilegedOperation,
        job_id: JobId,
    ) -> Result<OperationId, StoreError> {
        let id = operation.id;
        let mut w = self.write();
        let already = w.operations.contains_key(&id);
        w.operations.insert(id, operation.clone());
        if !already {
            w.operations_by_job.entry(job_id).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_operation(&self, id: OperationId) -> Result<PrivilegedOperation, StoreError> {
        self.read()
            .operations
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("operation {id}")))
    }

    async fn update_operation(
        &self,
        id: OperationId,
        updated: &PrivilegedOperation,
    ) -> Result<(), StoreError> {
        let mut w = self.write();
        let prev = w
            .operations
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("operation {id}")))?;
        let was_executing = matches!(prev.authorization, AuthorizationState::Executing);
        let now_executing = matches!(updated.authorization, AuthorizationState::Executing);
        w.operations.insert(id, updated.clone());
        match (was_executing, now_executing) {
            (false, true) => w.operations_executing.push(id),
            (true, false) => w.operations_executing.retain(|op| *op != id),
            _ => {}
        }
        Ok(())
    }

    async fn list_executing_operations(&self) -> Result<Vec<OperationId>, StoreError> {
        Ok(self.read().operations_executing.clone())
    }

    async fn list_operations_for_job(&self, job_id: JobId) -> Result<Vec<OperationId>, StoreError> {
        Ok(self
            .read()
            .operations_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }
}

impl ArtifactMetadataStore for MockStore {
    async fn put_artifact(&self, record: &ArtifactRecord) -> Result<ArtifactId, StoreError> {
        let id = record.id;
        let job = record.job_id;
        let mut w = self.write();
        let already = w.artifacts.contains_key(&id);
        w.artifacts.insert(id, record.clone());
        if !already {
            w.artifacts_by_job.entry(job).or_default().push(id);
        }
        Ok(id)
    }

    async fn get_artifact(&self, id: ArtifactId) -> Result<ArtifactRecord, StoreError> {
        self.read()
            .artifacts
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("artifact {id}")))
    }

    async fn artifact_exists(&self, id: ArtifactId) -> Result<bool, StoreError> {
        Ok(self.read().artifacts.contains_key(&id))
    }

    async fn list_artifacts_for_job(&self, job_id: JobId) -> Result<Vec<ArtifactId>, StoreError> {
        Ok(self
            .read()
            .artifacts_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }
}

impl WorkspaceMetadataStore for MockStore {
    async fn get_workspace_state(
        &self,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceState, StoreError> {
        self.read()
            .workspaces
            .get(handle)
            .cloned()
            .ok_or_else(|| StoreError::not_found(format!("workspace {handle}")))
    }

    async fn put_workspace_state(
        &self,
        state: &WorkspaceState,
        expected_revision: u64,
    ) -> Result<(), StoreError> {
        let mut w = self.write();
        match w.workspaces.get(&state.handle) {
            Some(existing) if existing.revision != expected_revision => {
                return Err(StoreError::conflict(format!(
                    "workspace {}: expected_revision={expected_revision}, found={}",
                    state.handle, existing.revision
                )));
            }
            None => {
                if expected_revision != 0 {
                    return Err(StoreError::conflict(format!(
                        "workspace {}: first write requires expected_revision=0, got {expected_revision}",
                        state.handle
                    )));
                }
            }
            _ => {}
        }
        let job = state.job_id;
        let already = w.workspaces.contains_key(&state.handle);
        w.workspaces.insert(state.handle.clone(), state.clone());
        if !already {
            w.workspaces_by_job
                .entry(job)
                .or_default()
                .push(state.handle.clone());
        }
        Ok(())
    }

    async fn list_workspaces_for_job(
        &self,
        job_id: JobId,
    ) -> Result<Vec<WorkspaceHandle>, StoreError> {
        Ok(self
            .read()
            .workspaces_by_job
            .get(&job_id)
            .cloned()
            .unwrap_or_default())
    }
}

/// Convenience constructor used by every smoke test in the suite.
pub fn store() -> MockStore {
    MockStore::new()
}
