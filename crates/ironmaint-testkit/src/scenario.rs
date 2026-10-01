//! The §101 scenario adapter — a test double, and the reason
//! `crates/ironmaint-testkit/tests/synthetic_e2e.rs` is not the
//! §101 driver.
//!
//! §101 needs a `DistributionAdapter` whose plan *changes as the
//! agent works*: the build fails at C1, the QA fails at C2, and
//! everything passes from C3 on. No production adapter can supply
//! that. `debian-stub` and `fedora-stub` plan real `debian.*` /
//! `fedora.*` capabilities for which no tool is registered in 0B,
//! and shipping a fixture adapter inside the daemon to make the
//! acceptance scenario run would be exactly the kind of stub the
//! phase forbids. §101 is explicitly a *synthetic* scenario, so the
//! double lives here, in the crate whose stated job is fakes and
//! fixtures.
//!
//! ## How the plan varies
//!
//! By **package version**, because that is what actually changes
//! between C1 and C2 in the scenario — the agent patches the source
//! and the next capture carries the next version. The adapter reads
//! `ctx.package.version` and swaps one tool key. Nothing else about
//! the plan moves, so a failure can only ever be the failure the
//! script asked for.
//!
//! ## Why one tool key serves many checks
//!
//! The `ironmaint-fixture` binary is keyed off its first
//! positional argument, and only eight keys exist. A walk to
//! `ReadyForApproval` needs twelve gates, so keys are reused across
//! stages. That is sound because a check is bound to its *evidence
//! kind* — `latest_evidence_for_check` matches on `kind`, not on
//! the tool — so twelve checks sharing four keys still aggregate
//! twelve independent verdicts. It would not be sound if a single
//! gate had two checks of the same kind, which is why the table
//! below lists each kind once.
//!
//! ## What this double deliberately does not do
//!
//! It does not execute anything. It produces plans; the real
//! subprocess is the `ironmaint-fixture` binary, reached through
//! `ProcessExecutor` exactly as the daemon reaches it.

use std::path::Path;
use std::sync::{Arc, Mutex};

use ironmaint_adapter_api::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, AdapterErrorKind, BuildCapability,
    BuildPlan, CandidateContext, ChangedPath, DistributionAdapter, IssueCapability,
    ObligationTemplate, PackageModelCapability, PathRole, PlannedCheck, PolicyCapability,
    PolicyContext, PolicyPlan, QaPlan, ReleaseCapability, ToolCapabilityKey, VersioningCapability,
    verdict_from_evidence_status,
};
use ironmaint_core::{
    AuthorityId, CandidateFingerprint, DistributionFamily, PackageName, PackageVersion,
};
use ironmaint_evidence::{ChangeDomain, EvidenceKind, GateStage};
use ironmaint_executor::{ExecutionClass, ExecutionLimits, ToolDefinitionRecord, ToolRegistry};
use ironmaint_policy::{
    Applicability, AuthorityClassification, ObligationOutcome, ObligationStrength, PolicyBaseline,
    PolicyReference,
};

/// A synthetic check that passes. Keyed off the fixture binary's
/// first positional argument.
pub const SYNTHETIC_BUILD_PASS: &str = "synthetic.build.validate";
/// A synthetic check that fails — a *tool* failure (exit 1), not an
/// infrastructure one.
pub const SYNTHETIC_BUILD_FAIL: &str = "synthetic.build.fail";
/// A synthetic QA check that passes.
pub const SYNTHETIC_QA_PASS: &str = "synthetic.qa.lintian";
/// A synthetic QA check that fails.
pub const SYNTHETIC_QA_FAIL: &str = "synthetic.qa.fail";
/// A synthetic policy evaluation that passes. §48's "deterministic
/// fixture policy evaluator" — the evidence it produces is what an
/// obligation's verdict is derived from.
pub const SYNTHETIC_POLICY_PASS: &str = "synthetic.policy.validate";
/// A synthetic policy evaluation that fails, and so leaves every
/// obligation it judges `Fail`.
pub const SYNTHETIC_POLICY_FAIL: &str = "synthetic.policy.fail";

/// Which gate stage the §101 walk needs a passing check for, and
/// the evidence kind that produces it.
///
/// Twelve entries, one per rule the walk traverses, in §20 order.
/// The stages are spelled out here rather than derived from
/// `TRANSITION_RULES` because a test double that reads the state
/// machine's requirements is a test double that cannot be wrong
/// about them — but a *driver* must not have to notice when a
/// requirement is added, so `covers_every_required_gate_stage`
/// cross-checks this list against the rules.
///
/// `LicenseReview` is the kind that reaches `FinalValidation`;
/// `Reproducibility` reaches the same stage and is not listed,
/// because a gate aggregates every check attached to it and one is
/// enough.
pub const SCENARIO_STAGES: &[(EvidenceKind, GateStage)] = &[
    (
        EvidenceKind::SourcePreparation,
        GateStage::SourcePreparation,
    ),
    (EvidenceKind::SourceIntegrity, GateStage::SourceAnalysis),
    (EvidenceKind::IssueCorrelation, GateStage::IssueAnalysis),
    (EvidenceKind::Maintenance, GateStage::Maintenance),
    (EvidenceKind::PolicyEvaluation, GateStage::PolicyEvaluation),
    (EvidenceKind::Build, GateStage::BuildValidation),
    (EvidenceKind::PackageQa, GateStage::PackageQa),
    (
        EvidenceKind::FunctionalTest,
        GateStage::FunctionalValidation,
    ),
    (EvidenceKind::UpgradeTest, GateStage::UpgradeValidation),
    (EvidenceKind::ReleaseReview, GateStage::ReleaseReview),
    (EvidenceKind::ReleaseAssembly, GateStage::CandidateAssembly),
    (EvidenceKind::LicenseReview, GateStage::FinalValidation),
];

/// The synthetic tool keys a §101 run registers, in the order
/// [`register_scenario_tools`] installs them.
#[derive(Debug, Clone)]
pub struct ScenarioTools {
    pub build_pass: ToolCapabilityKey,
    pub build_fail: ToolCapabilityKey,
    pub qa_pass: ToolCapabilityKey,
    pub qa_fail: ToolCapabilityKey,
    pub policy_pass: ToolCapabilityKey,
    pub policy_fail: ToolCapabilityKey,
}

/// Register the six §92 fixture tools plus the four extra keys the
/// scenario needs: `synthetic.qa.*` and `synthetic.policy.*`.
///
/// The four are deliberately *not* added to
/// `ironmaint_synthetic_tools::register_synthetic_tools`: that set
/// is the §92 exit checkpoint and the daemon registers it verbatim.
/// Widening the daemon's registry to serve a test double would make
/// the production tool surface depend on the acceptance scenario,
/// which is the wrong direction for a dependency.
///
/// # Errors
///
/// Returns the adapter-api-shaped error when a key is malformed or
/// the registry rejects a record.
pub fn register_scenario_tools(
    registry: &mut ToolRegistry,
    fixture_bin: &Path,
) -> Result<ScenarioTools, AdapterError> {
    // The two `build.*` keys the scenario uses are among the six the
    // §92 set registers; take them from there rather than defining
    // them twice.
    let six = ironmaint_synthetic_tools::register_synthetic_tools(registry, fixture_bin)
        .map_err(|e| AdapterError::new(AdapterErrorKind::InternalAdapterFailure, e.to_string()))?;
    if six.validate.as_str() != SYNTHETIC_BUILD_PASS || six.fail.as_str() != SYNTHETIC_BUILD_FAIL {
        return Err(AdapterError::new(
            AdapterErrorKind::InternalAdapterFailure,
            "the §92 fixture set no longer defines the keys the scenario plans against",
        ));
    }
    let qa_pass = register_one(registry, SYNTHETIC_QA_PASS, fixture_bin)?;
    let qa_fail = register_one(registry, SYNTHETIC_QA_FAIL, fixture_bin)?;
    let policy_pass = register_one(registry, SYNTHETIC_POLICY_PASS, fixture_bin)?;
    let policy_fail = register_one(registry, SYNTHETIC_POLICY_FAIL, fixture_bin)?;
    Ok(ScenarioTools {
        build_pass: six.validate,
        build_fail: six.fail,
        qa_pass,
        qa_fail,
        policy_pass,
        policy_fail,
    })
}

fn register_one(
    registry: &mut ToolRegistry,
    key: &str,
    fixture_bin: &Path,
) -> Result<ToolCapabilityKey, AdapterError> {
    let capability = ToolCapabilityKey::new(key).map_err(|e| {
        AdapterError::new(
            AdapterErrorKind::InvalidConfiguration,
            format!("{key}: {e}"),
        )
    })?;
    let record = ToolDefinitionRecord::new(
        capability.clone(),
        fixture_bin,
        vec![std::ffi::OsString::from(key)],
        ExecutionClass::Check,
        ExecutionLimits::default(),
    );
    registry.register(Box::new(record)).map_err(|e| {
        AdapterError::new(
            AdapterErrorKind::InternalAdapterFailure,
            format!("{key}: {e}"),
        )
    })?;
    Ok(capability)
}

/// The requirement text of the scenario's one policy obligation.
///
/// `RuntimeCommand::RecordObligationOutcome` addresses an obligation
/// by its `requirement` string, so the driver that records §101
/// step 18's failure and step 24's pass needs this exact text. It
/// is exported rather than retyped so the two cannot drift. A
/// tool-only driver never types it at all — it runs
/// `synthetic.policy.*` and the runtime asks the adapter for the
/// verdict, which is the point of
/// [`PolicyCapability::evaluate_obligation`].
pub const SCENARIO_OBLIGATION_REQUIREMENT: &str =
    "the source is the version the distribution ships";

/// Which package versions fail which check.
///
/// The scenario's own shape, expressed as data so a caller can
/// script a different one — and so the §101 driver is reading its
/// expectations from one place rather than hard-coding "1.0.0" in
/// six different spots.
#[derive(Debug, Clone, Default)]
pub struct ScenarioScript {
    /// Versions whose *build* check is scripted to fail.
    pub build_failures: Vec<String>,
    /// Versions whose *QA* check is scripted to fail.
    pub qa_failures: Vec<String>,
    /// Versions whose *policy* check is scripted to fail.
    ///
    /// This is how §101 step 18 happens on a tool surface: there
    /// is no `RecordObligationOutcome` tool — §53 forbids
    /// `ironmaint_obligation_set_pass` — so the only way an agent
    /// can leave a mandatory obligation failing is to run the
    /// policy evaluator and have its evidence come back negative.
    pub policy_failures: Vec<String>,
    /// Whether the three lists above are matched against the
    /// package *version* or against the *ordinal of the capture*.
    ///
    /// See [`ScenarioScript::capture_ordinal`]. `false` — the
    /// version — is what §101 describes and what a driver holding
    /// the command surface can produce, because it builds the
    /// `SourceCandidate` itself and names any version it likes.
    pub by_capture_ordinal: bool,
    /// First-seen order of the candidates this script has been
    /// asked about, keyed by fingerprint.
    ///
    /// Empty for a version-keyed script, and never read.
    capture_order: Arc<Mutex<Vec<CandidateFingerprint>>>,
}

impl ScenarioScript {
    /// The script §101 describes, for a driver holding the command
    /// surface: the first capture fails to build, the second builds
    /// but fails QA, the third and fourth are clean, and the
    /// obligation's verdict is written with
    /// `RuntimeCommand::RecordObligationOutcome`.
    ///
    /// Every check passes at C3, so the walk gets all the way to
    /// rule 12 and stops there — the isolated shape of "a failed
    /// mandatory obligation is the only thing holding the job
    /// back".
    #[must_use]
    pub fn section_101() -> Self {
        Self {
            build_failures: vec!["1.0.0".to_string()],
            qa_failures: vec!["1.0.1".to_string()],
            policy_failures: Vec::new(),
            by_capture_ordinal: false,
            capture_order: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The same script for a driver holding **only** the MCP tool
    /// surface, where the obligation's verdict is derived from the
    /// policy evaluator's evidence rather than written.
    ///
    /// C3's policy check fails too. That is not a deviation from
    /// §101 — step 18 says the *obligation* fails, and it fails
    /// because its evaluator produced failing evidence — but it
    /// does mean the walk stops at rule 5 (`SourceRevision →
    /// SourceIntegrity`, which requires the `PolicyEvaluation`
    /// gate) as well as at rule 12, and C4 repairs both.
    /// [`ScenarioScript::section_101`] is the script that isolates
    /// rule 12; this is the one a tool-only driver can run at all.
    #[must_use]
    pub fn section_101_policy_derived() -> Self {
        Self {
            policy_failures: vec!["1.0.2".to_string()],
            ..Self::section_101()
        }
    }

    /// §101 keyed by capture order rather than by version, for a
    /// driver that holds **only** the MCP tool surface.
    ///
    /// ## Why a second key is needed at all
    ///
    /// `candidate.capture` names the version itself, and it names it
    /// `0+ironmaint` every time: the workspace mints that sentinel
    /// because a real upstream version is not resolvable in 0B
    /// (`crates/ironmaint-workspace/src/capture.rs`). The tool
    /// takes no version argument, so a tool-only driver cannot
    /// produce the input a version-keyed plan varies on.
    ///
    /// That is the same shape as the four missing links 0B.10 C1
    /// found — a component that is correct, and an input nothing
    /// produces — except that here the missing thing is a value the
    /// tool surface does not expose rather than a writer that does
    /// not exist. The fix is deliberately *not* to widen
    /// `CaptureInput`: a version is a claim about upstream, and
    /// letting a caller assert one would put a caller-supplied
    /// string into a `CandidateFingerprint`. So the script varies
    /// on something the agent does control and the capture does
    /// change: how many distinct sources this adapter has been
    /// asked to plan for.
    ///
    /// ## What this gives up
    ///
    /// The claim is weaker. "The third candidate I am asked to plan
    /// for gets the third treatment" is not "the source at version
    /// 1.0.2 fails policy". A capture-ordinal key says the
    /// *sequence* of the scenario and nothing about the source
    /// being repaired — if the agent captured twice without
    /// patching, this script would still hand out the third
    /// treatment. [`ScenarioScript::section_101`] is keyed the way
    /// §101 actually describes, and is the script to read if you
    /// are asking what the scenario asserts.
    #[must_use]
    pub fn capture_ordinal() -> Self {
        Self {
            build_failures: vec!["0".to_string()],
            qa_failures: vec!["1".to_string()],
            policy_failures: vec!["2".to_string()],
            by_capture_ordinal: true,
            capture_order: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// This candidate's key: its version, or its position in the
    /// sequence of candidates this script has been asked about.
    ///
    /// Keyed on the fingerprint rather than on a counter so that
    /// re-planning the same candidate — which
    /// `derive_candidate_plans` does guard against, but a
    /// correctness that should not be load-bearing — yields the
    /// same key rather than the next one.
    #[must_use]
    pub fn key_for(&self, candidate: &ironmaint_core::SourceCandidate) -> String {
        if !self.by_capture_ordinal {
            return candidate.package().version.as_str().to_string();
        }
        let fingerprint = candidate.fingerprint();
        // A poisoned lock means some other thread panicked while
        // holding it. The value is a plain `Vec` this method is the
        // only writer to, so recovering it is strictly better than
        // propagating a panic into a test double.
        let mut order = self
            .capture_order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match order.iter().position(|f| f == fingerprint) {
            Some(at) => at.to_string(),
            None => {
                order.push(fingerprint.clone());
                (order.len() - 1).to_string()
            }
        }
    }
}

/// A `DistributionAdapter` double whose check plan varies by
/// package version, driving the §101 acceptance scenario.
///
/// See the module docs for why this exists and why it is here.
#[derive(Debug, Clone)]
pub struct ScenarioAdapter {
    family: DistributionFamily,
    script: ScenarioScript,
    build_pass: ToolCapabilityKey,
    build_fail: ToolCapabilityKey,
    qa_pass: ToolCapabilityKey,
    qa_fail: ToolCapabilityKey,
    policy_pass: ToolCapabilityKey,
    policy_fail: ToolCapabilityKey,
}

impl ScenarioAdapter {
    /// Build an adapter for `family` with an explicit script.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterErrorKind::InvalidConfiguration`](AdapterErrorKind)
    /// if one of the four synthetic tool keys is not well-formed.
    /// The keys are parsed here rather than per plan call so that
    /// [`ScenarioAdapter::tool_for`] and
    /// [`ScenarioAdapter::plan_for`] are infallible and a driver
    /// never has to unwrap a planning decision.
    pub fn new(family: DistributionFamily, script: ScenarioScript) -> Result<Self, AdapterError> {
        Ok(Self {
            family,
            script,
            build_pass: parse_key(SYNTHETIC_BUILD_PASS)?,
            build_fail: parse_key(SYNTHETIC_BUILD_FAIL)?,
            qa_pass: parse_key(SYNTHETIC_QA_PASS)?,
            qa_fail: parse_key(SYNTHETIC_QA_FAIL)?,
            policy_pass: parse_key(SYNTHETIC_POLICY_PASS)?,
            policy_fail: parse_key(SYNTHETIC_POLICY_FAIL)?,
        })
    }

    /// The adapter §101 asks for, in the `synthetic` family.
    ///
    /// A family of its own rather than `debian` is deliberate: the
    /// scenario must not be able to pass by accidentally matching a
    /// real adapter, and a real adapter would plan capabilities
    /// whose tools do not exist in 0B.
    ///
    /// # Errors
    ///
    /// As [`ScenarioAdapter::new`].
    pub fn section_101() -> Result<Self, AdapterError> {
        Self::new(
            DistributionFamily::new(SYNTHETIC_FAMILY).map_err(|e| {
                AdapterError::new(AdapterErrorKind::InvalidConfiguration, e.to_string())
            })?,
            ScenarioScript::section_101(),
        )
    }

    /// The §101 script for a driver holding only the MCP tool
    /// surface — see [`ScenarioScript::capture_ordinal`] for why
    /// this one is keyed by capture order rather than by version,
    /// and why the policy check has to fail alongside the
    /// obligation.
    ///
    /// # Errors
    ///
    /// Propagates [`ScenarioAdapter::new`].
    pub fn section_101_capture_ordinal() -> Result<Self, AdapterError> {
        Self::new(
            DistributionFamily::new(SYNTHETIC_FAMILY).map_err(|e| {
                AdapterError::new(AdapterErrorKind::InvalidConfiguration, e.to_string())
            })?,
            ScenarioScript::capture_ordinal(),
        )
    }

    /// The script this adapter is running, for a driver that wants
    /// to assert against the same data the adapter planned from.
    #[must_use]
    pub fn script(&self) -> &ScenarioScript {
        &self.script
    }

    /// The tool key a check of `kind` runs for the script key
    /// `key`.
    ///
    /// `key` is a package version or a capture ordinal depending on
    /// [`ScenarioScript::by_capture_ordinal`] — resolve it with
    /// [`ScenarioScript::key_for`] rather than passing a version,
    /// or the two drivers' scripts will not mean the same thing.
    ///
    /// Exposed so the driver can assert "the build check for 1.0.0
    /// is the one that fails" without duplicating the rule.
    #[must_use]
    pub fn tool_for(&self, kind: &EvidenceKind, key: &str) -> ToolCapabilityKey {
        if kind == &EvidenceKind::Build && self.script.build_failures.iter().any(|v| v == key) {
            self.build_fail.clone()
        } else if kind == &EvidenceKind::PackageQa
            && self.script.qa_failures.iter().any(|v| v == key)
        {
            self.qa_fail.clone()
        } else if kind == &EvidenceKind::PolicyEvaluation {
            if self.script.policy_failures.iter().any(|v| v == key) {
                self.policy_fail.clone()
            } else {
                self.policy_pass.clone()
            }
        } else if is_qa_stage(kind) {
            self.qa_pass.clone()
        } else {
            self.build_pass.clone()
        }
    }

    /// The twelve mandatory checks a §101 walk needs, planned for
    /// `candidate`.
    #[must_use]
    pub fn plan_for(&self, candidate: &ironmaint_core::SourceCandidate) -> Vec<PlannedCheck> {
        let key = self.script.key_for(candidate);
        SCENARIO_STAGES
            .iter()
            .map(|(kind, _)| PlannedCheck::new(self.tool_for(kind, &key), kind.clone(), true))
            .collect()
    }
}

fn parse_key(literal: &str) -> Result<ToolCapabilityKey, AdapterError> {
    ToolCapabilityKey::new(literal).map_err(|e| {
        AdapterError::new(
            AdapterErrorKind::InvalidConfiguration,
            format!("{literal}: {e}"),
        )
    })
}

fn is_qa_stage(kind: &EvidenceKind) -> bool {
    matches!(
        kind,
        EvidenceKind::PackageQa | EvidenceKind::FunctionalTest | EvidenceKind::UpgradeTest
    )
}

/// The distribution family the scenario adapter claims.
pub const SYNTHETIC_FAMILY: &str = "synthetic";

impl DistributionAdapter for ScenarioAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            family: self.family.clone(),
            implementation_name: "scenario-double".into(),
            implementation_version: "0.0.0".into(),
            capabilities: AdapterCapabilities::new(),
        }
    }

    fn versioning(&self) -> &dyn VersioningCapability {
        self
    }

    fn package_model(&self) -> &dyn PackageModelCapability {
        self
    }

    fn policy(&self) -> Option<&dyn PolicyCapability> {
        Some(self)
    }

    fn build(&self) -> Option<&dyn BuildCapability> {
        Some(self)
    }

    fn issues(&self) -> Option<&dyn IssueCapability> {
        None
    }

    fn release(&self) -> Option<&dyn ReleaseCapability> {
        None
    }
}

impl BuildCapability for ScenarioAdapter {
    fn build_plan(&self, ctx: &CandidateContext) -> Result<BuildPlan, AdapterError> {
        let mut plan = BuildPlan::new();
        for check in self.plan_for(ctx.candidate) {
            plan = plan.with_check(check);
        }
        Ok(plan)
    }

    fn qa_plan(&self, _ctx: &CandidateContext) -> Result<QaPlan, AdapterError> {
        // Every check is planned in one pass by `build_plan`. §45
        // splits build and QA into two bundles because real
        // adapters run them at different times; the scenario runs
        // them together, so an empty QA plan keeps the driver's
        // view honest: nothing it runs is unaccounted for in
        // `BuildPlan`.
        Ok(QaPlan::new())
    }
}

impl PolicyCapability for ScenarioAdapter {
    fn authority_order(&self) -> Vec<AuthorityClassification> {
        vec![AuthorityClassification::NormativePolicy]
    }

    fn derive_obligation_plan(&self, ctx: &PolicyContext) -> Result<PolicyPlan, AdapterError> {
        // One mandatory obligation, referencing the version it
        // applies to. §49's diagram has the agent change
        // `policy_version` when patching; naming it here means the
        // obligation recorded for C4 is visibly a different policy
        // statement from the one recorded for C3, rather than the
        // same row with a different status.
        let key = self.script.key_for(ctx.candidate);
        let template = ObligationTemplate::new(
            PolicyReference::new(AuthorityId::new()).with_section(&key),
            ObligationStrength::Mandatory,
            Applicability::Applicable,
            SCENARIO_OBLIGATION_REQUIREMENT,
        );
        Ok(PolicyPlan {
            baseline: PolicyBaseline::new(ctx.package.package.distribution.clone()),
            obligation_templates: vec![template],
        })
    }

    /// §48: the obligation's verdict is the policy evaluator's
    /// verdict, and by nothing else. A tool-only driver never
    /// writes it, so the only way this obligation can fail is by
    /// the evidence coming back `Fail`.
    fn evaluate_obligation(
        &self,
        _context: &PolicyContext,
        obligation: &ObligationTemplate,
        evidence: &ironmaint_evidence::Evidence,
    ) -> Result<ObligationOutcome, AdapterError> {
        if obligation.requirement != SCENARIO_OBLIGATION_REQUIREMENT {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidConfiguration,
                format!(
                    "`{}` is not an obligation the scenario adapter derives",
                    obligation.requirement
                ),
            ));
        }
        verdict_from_evidence_status(evidence.status)
    }
}

impl VersioningCapability for ScenarioAdapter {
    /// Accept any version core already validated.
    ///
    /// The scenario's versions are opaque strings from core's point
    /// of view, and inventing a grammar here would assert a
    /// distribution-neutral fact about `1.0.0` that no spec makes.
    fn validate(&self, version: &PackageVersion) -> Result<(), AdapterError> {
        if version.as_str().is_empty() {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidVersion,
                "empty version",
            ));
        }
        Ok(())
    }

    /// Compare dotted numeric components numerically, falling back
    /// to lexicographic for any non-numeric tail.
    ///
    /// Enough for `1.0.0 < 1.0.1 < 1.0.2 < 1.0.3`, which is all the
    /// scenario asserts, and honest about being a scenario ordering
    /// rather than a Debian or RPM one — real ordering belongs to
    /// the distribution adapters, and §0A is explicit that core must
    /// not grow any.
    fn compare(
        &self,
        left: &PackageVersion,
        right: &PackageVersion,
    ) -> Result<std::cmp::Ordering, AdapterError> {
        let rank = |v: &PackageVersion| -> Vec<u64> {
            v.as_str()
                .split(['.', '-'])
                .map(|part| part.parse::<u64>().unwrap_or(u64::MAX))
                .collect()
        };
        let (l, r) = (rank(left), rank(right));
        let len = l.len().max(r.len());
        for i in 0..len {
            let a = l.get(i).copied().unwrap_or(0);
            let b = r.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                std::cmp::Ordering::Equal => {}
                other => return Ok(other),
            }
        }
        // Equal numerically: fall back to the raw strings so
        // `1.0` and `1.0.0` still have a stable order rather than
        // comparing equal.
        Ok(left.as_str().cmp(right.as_str()))
    }
}

impl PackageModelCapability for ScenarioAdapter {
    fn validate_name(&self, name: &PackageName) -> Result<(), AdapterError> {
        if name.as_str().is_empty() {
            return Err(AdapterError::new(
                AdapterErrorKind::InvalidPackage,
                "empty package name",
            ));
        }
        Ok(())
    }

    /// Map each hint onto the change domain it belongs to,
    /// deduplicated.
    ///
    /// `PathRole` is `#[non_exhaustive]`, so a role added after
    /// this was written falls to the catch-all and becomes
    /// `AdapterSpecific(<its name>)` rather than being dropped.
    /// Dropping it would under-report what a patch touched, and
    /// under-reporting is the failure mode §31's propagation rules
    /// would then miss.
    fn classify_changes(&self, changes: &[ChangedPath]) -> Result<Vec<ChangeDomain>, AdapterError> {
        let mut out: Vec<ChangeDomain> = Vec::new();
        for change in changes {
            let domain = match &change.role {
                PathRole::UpstreamSource => ChangeDomain::UpstreamSource,
                PathRole::UpstreamTests | PathRole::PackagingTests => ChangeDomain::Tests,
                PathRole::PackagingMetadata => ChangeDomain::PackagingMetadata,
                PathRole::PackagingBuildConfig => ChangeDomain::BuildConfiguration,
                PathRole::PackagingRuntimeDeps => ChangeDomain::RuntimeDependencies,
                PathRole::PackagingPatch => ChangeDomain::PatchSet,
                PathRole::LicensingMetadata => ChangeDomain::LicensingMetadata,
                PathRole::DocumentationOnly => ChangeDomain::DocumentationOnly,
                PathRole::ReleaseMetadata => ChangeDomain::ReleaseMetadata,
                PathRole::VendorSpecific(name) => ChangeDomain::AdapterSpecific(name.clone()),
                other => ChangeDomain::AdapterSpecific(format!("{other:?}")),
            };
            if !out.contains(&domain) {
                out.push(domain);
            }
        }
        Ok(out)
    }
}

// A test module asserts things with `assert!` and needs to
// unwrap the constructors it just built; the panic lints exist to
// keep panicking code out of *shipped* paths, and this is not one.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn adapter() -> ScenarioAdapter {
        ScenarioAdapter::section_101().expect("the scenario's keys are well-formed")
    }

    /// A candidate at `version` over the tree `n`.
    ///
    /// The tree varies because it has to: a candidate's fingerprint
    /// covers the tree OID, so two candidates differing only in
    /// version are different candidates but two candidates
    /// differing in *nothing* are the same one — and a
    /// fingerprint-keyed script would hand them the same ordinal.
    fn candidate(version: &str, n: char) -> ironmaint_core::SourceCandidate {
        use ironmaint_core::{
            DistributionRef, DistributionRelease, GitHashAlgorithm, GitObjectId, JobId,
            PackageIdentity, PackageRevision, RepositoryRef, SourceCandidate, VcsKind,
        };
        let family = DistributionFamily::new(SYNTHETIC_FAMILY).unwrap();
        let release = DistributionRelease::new("unstable").unwrap();
        let package = PackageIdentity::new(
            DistributionRef::new(family, release),
            PackageName::new("synthetic-pkg").unwrap(),
        );
        let repository = RepositoryRef::new(
            VcsKind::Git,
            url::Url::parse("https://example.invalid/synthetic-pkg.git").unwrap(),
        )
        .unwrap();
        let object = |c: char| GitObjectId::new(GitHashAlgorithm::Sha1, c.to_string().repeat(40));
        SourceCandidate::new(
            JobId::new(),
            PackageRevision::new(package, PackageVersion::new(version).unwrap()),
            repository,
            object('a').unwrap(),
            object(n).unwrap(),
            time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        )
    }

    #[test]
    fn the_plan_covers_every_required_gate_stage() {
        // The whole point of `SCENARIO_STAGES` is that the driver's
        // twelve subprocesses satisfy the twelve rules. If a rule
        // is added, this fails and names it.
        let reachable: std::collections::HashSet<GateStage> =
            SCENARIO_STAGES.iter().map(|(_, stage)| *stage).collect();
        let mut missing = Vec::new();
        for rule in ironmaint_state::TRANSITION_RULES {
            for stage in rule.requirements.gates {
                if *stage == GateStage::Publication {
                    // Past `ReadyForApproval`; §101 stops before it
                    // and step 29 asserts nothing is published.
                    continue;
                }
                if !reachable.contains(stage) {
                    missing.push(format!(
                        "{} -> {} requires `{}`",
                        rule.from.name(),
                        rule.to.name(),
                        stage.name(),
                    ));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "the scenario adapter plans no check for:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn the_scripted_failures_are_the_only_failures() {
        let a = adapter();
        for (version, failing_kind) in [
            ("1.0.0", EvidenceKind::Build),
            ("1.0.1", EvidenceKind::PackageQa),
        ] {
            for (kind, _) in SCENARIO_STAGES {
                let key = a.tool_for(kind, version);
                let fails =
                    key.as_str() == SYNTHETIC_BUILD_FAIL || key.as_str() == SYNTHETIC_QA_FAIL;
                assert_eq!(
                    fails,
                    *kind == failing_kind,
                    "at {version}, {kind} -> {} but only {failing_kind} should fail",
                    key.as_str()
                );
            }
        }
    }

    #[test]
    fn every_planned_check_is_mandatory_and_has_a_tool() {
        let a = adapter();
        for version in ["1.0.0", "1.0.1", "1.0.2", "1.0.3"] {
            let plan = a.plan_for(&candidate(version, 'b'));
            assert_eq!(plan.len(), SCENARIO_STAGES.len());
            for check in plan {
                assert!(check.mandatory, "{version}: {} is not mandatory", check.key);
                assert_eq!(
                    check.key.as_str(),
                    a.tool_for(&check.evidence_kind, version).as_str(),
                    "{version}: the plan and the rule disagree about {kind}",
                    kind = check.evidence_kind
                );
            }
        }
    }

    #[test]
    fn every_planned_evidence_kind_reaches_a_gate() {
        // `Other(_)` maps to `None` and is filtered out of
        // materialisation, so a check carrying one would silently
        // vanish and the walk would stall on a `MissingGate`
        // nobody could explain.
        for (kind, stage) in SCENARIO_STAGES {
            assert!(
                ironmaint_runtime::check::gate_stage_for(kind) == Some(*stage),
                "{kind} should reach {stage}"
            );
        }
    }

    /// The capture-ordinal script has to hand out a *different*
    /// treatment per capture, and has to keep handing out the same
    /// one when the runtime asks twice about the same candidate —
    /// `plan_for` is reached from both `build_plan` and the
    /// driver's own assertions.
    #[test]
    fn the_capture_ordinal_script_varies_by_capture_not_by_version() {
        let a =
            ScenarioAdapter::section_101_capture_ordinal().expect("the scenario's keys are valid");
        // Four candidates at the *same* version, which is exactly
        // what `candidate.capture` produces: the workspace mints
        // `0+ironmaint` every time and only the tree differs.
        let cands: Vec<_> = ['b', 'c', 'd', 'e']
            .iter()
            .map(|n| candidate("0+ironmaint", *n))
            .collect();

        assert_eq!(
            a.tool_for(&EvidenceKind::Build, &a.script().key_for(&cands[0]))
                .as_str(),
            SYNTHETIC_BUILD_FAIL,
            "capture 0 fails its build"
        );
        assert_eq!(
            a.tool_for(&EvidenceKind::Build, &a.script().key_for(&cands[1]))
                .as_str(),
            SYNTHETIC_BUILD_PASS,
            "capture 1 builds; the version did not change, only the ordinal"
        );
        assert_eq!(
            a.tool_for(&EvidenceKind::PackageQa, &a.script().key_for(&cands[1]))
                .as_str(),
            SYNTHETIC_QA_FAIL,
            "capture 1 fails its QA"
        );
        assert_eq!(
            a.tool_for(
                &EvidenceKind::PolicyEvaluation,
                &a.script().key_for(&cands[2])
            )
            .as_str(),
            SYNTHETIC_POLICY_FAIL,
            "capture 2 is the one whose policy evaluation fails, which is what              makes its mandatory obligation fail"
        );
        assert_eq!(
            a.tool_for(
                &EvidenceKind::PolicyEvaluation,
                &a.script().key_for(&cands[3])
            )
            .as_str(),
            SYNTHETIC_POLICY_PASS,
            "capture 3 is clean"
        );
    }

    #[test]
    fn a_replanned_candidate_keeps_its_ordinal() {
        let a =
            ScenarioAdapter::section_101_capture_ordinal().expect("the scenario's keys are valid");
        let first = candidate("1.0.0", 'b');
        let second = candidate("1.0.0", 'c');
        assert_eq!(a.script().key_for(&first), "0");
        assert_eq!(a.script().key_for(&second), "1");
        // Asking again about either must not consume a new ordinal,
        // or the second `build_plan` for a candidate would plan it
        // differently from the first.
        assert_eq!(a.script().key_for(&first), "0");
        assert_eq!(a.script().key_for(&second), "1");
    }

    #[test]
    fn version_comparison_orders_the_scenario_versions() {
        let a = adapter();
        let v = |s: &str| PackageVersion::new(s).unwrap();
        use std::cmp::Ordering;
        assert_eq!(a.compare(&v("1.0.0"), &v("1.0.1")).unwrap(), Ordering::Less);
        assert_eq!(
            a.compare(&v("1.0.3"), &v("1.0.2")).unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            a.compare(&v("1.0.2"), &v("1.0.2")).unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn change_classification_deduplicates() {
        let a = adapter();
        let domains = a
            .classify_changes(&[
                ChangedPath::new("a.patch", PathRole::PackagingPatch),
                ChangedPath::new("b.patch", PathRole::PackagingPatch),
                ChangedPath::new("docs/README", PathRole::DocumentationOnly),
            ])
            .unwrap();
        assert_eq!(
            domains,
            vec![ChangeDomain::PatchSet, ChangeDomain::DocumentationOnly]
        );
    }
}
