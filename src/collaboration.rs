//! Immutable authoring snapshots for structured FluxR modules.

use crate::structured::{
    BodyChecker, Expr, Invocation, Limits, Module, Source, Stmt, Type, Value, Workspace,
};
use crate::{Error, bound};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

mod query;
pub use query::{BehaviorImpact, DeclarationDependencies, DeclarationKey, DependencyManifest};
mod versioned;
pub use versioned::{
    ProgramVersion, VersionedActivation, VersionedCandidate, VersionedProgram, VersionedSnapshot,
};

type BehaviorMemo = Arc<OnceLock<Result<Revision, String>>>;

const MAX_CHANGES: usize = 1024;
const MAX_EDITS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision([u8; 32]);

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Revision {
    type Err = Error;

    fn from_str(hex: &str) -> Result<Self, Error> {
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error("expected a 64-digit revision".into()));
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                .map_err(|_| Error("invalid revision".into()))?;
        }
        Ok(Self(bytes))
    }
}

fn digest(value: &impl Serialize) -> Result<Revision, Error> {
    let (revision, bytes) = measured_digest(value)?;
    bound(bytes)?;
    Ok(revision)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub function: String,
    #[serde(default)]
    pub receiver: Option<String>,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Exclusive,
    /// A zero-argument F32 export with a literal nonnegative baseline.
    MinF32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub author: String,
    #[serde(default)]
    pub requires: BTreeMap<String, Revision>,
    /// All edits in a change are admitted or held together.
    pub edits: Vec<Edit>,
}

/// Additive opt-in protection for assumptions about a declaration's behavior.
/// Generate `behavior_requires` with `Snapshot::behavior_read_set` at authoring
/// time. `Change::requires` remains a declaration-only pin and is never weakened.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckedChange {
    pub change: Change,
    #[serde(default)]
    pub behavior_requires: BTreeMap<String, Revision>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    ReplaceBody {
        target: String,
        expected: Revision,
        body: Vec<Stmt>,
    },
    MinF32 {
        target: String,
        expected: Revision,
        bits: u32,
    },
}

impl Edit {
    fn target(&self) -> &str {
        match self {
            Self::ReplaceBody { target, .. } | Self::MinF32 { target, .. } => target,
        }
    }

    fn expected(&self) -> Revision {
        match self {
            Self::ReplaceBody { expected, .. } | Self::MinF32 { expected, .. } => *expected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", content = "detail", rename_all = "snake_case")]
pub enum Rejection {
    Stale(String),
    Invalid(String),
    Conflict(String),
    DependencyChanged(String),
}

#[derive(Clone)]
pub struct Snapshot {
    source: Arc<Source>,
    module: Arc<Module>,
    targets: Arc<BTreeMap<String, (usize, Mode)>>,
    contracts: Arc<BTreeMap<String, Revision>>,
    revisions: Arc<BTreeMap<String, Revision>>,
    dependencies: Arc<Vec<BTreeSet<usize>>>,
    // Immutable, snapshot-local indexes. Querying a small reverse closure must
    // not rebuild the graph or rehash every declaration in the module.
    reverse_dependencies: Arc<Vec<BTreeSet<usize>>>,
    target_keys: Arc<Vec<Option<String>>>,
    declaration_revisions: Arc<Vec<Revision>>,
    environment: Revision,
    // Each immutable generation shares only memo entries whose complete call
    // closure is unchanged. An entry is initialized once across parallel readers.
    behavior_cache: Arc<Vec<BehaviorMemo>>,
    id: Revision,
}

impl Snapshot {
    /// Bind stable IDs once. Function order and spellings in existing source
    /// remain compatible with the structured module format.
    pub fn new(source: Source, targets: Vec<Target>) -> Result<Self, Error> {
        // Validate borrowed input before cloning, hashing target declarations,
        // or retaining a snapshot. The compiled canonical form owns its bytes.
        let module = Arc::new(Module::compile_borrowed(&source)?);
        let declarations: BTreeMap<_, _> = source
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| {
                (
                    (function.receiver.as_deref(), function.name.as_str()),
                    index,
                )
            })
            .collect();
        let mut slots = BTreeMap::new();
        let mut contracts = BTreeMap::new();
        let mut registered = BTreeSet::new();
        for target in targets {
            let index = declarations
                .get(&(target.receiver.as_deref(), target.function.as_str()))
                .copied()
                .ok_or_else(|| Error(format!("unknown declaration {}", target.function)))?;
            let function = &source.functions[index];
            if target.id.is_empty() || slots.contains_key(&target.id) || !registered.insert(index) {
                return Err(Error(format!("duplicate or empty target {}", target.id)));
            }
            if matches!(target.mode, Mode::MinF32)
                && (!function.export
                    || function.receiver.is_some()
                    || !function.params.is_empty()
                    || function.result != Type::F32
                    || literal_limit(&function.body).is_none())
            {
                return Err(Error(format!("invalid min_f32 hook {}", target.id)));
            }
            contracts.insert(target.id.clone(), digest(&(&target, function))?);
            slots.insert(target.id, (index, target.mode));
        }
        Self::build(source, module, Arc::new(slots), Arc::new(contracts))
    }

    fn build(
        source: Source,
        module: Arc<Module>,
        targets: Arc<BTreeMap<String, (usize, Mode)>>,
        contracts: Arc<BTreeMap<String, Revision>>,
    ) -> Result<Self, Error> {
        let id = snapshot_id(&module, &contracts)?;
        let declaration_revisions = source
            .functions
            .iter()
            .map(behavior_digest)
            .collect::<Result<Vec<_>, Error>>()?;
        let revisions = targets
            .iter()
            .map(|(key, &(index, _))| (key.clone(), declaration_revisions[index]))
            .collect();
        let mut target_keys = vec![None; source.functions.len()];
        for (key, &(index, _)) in targets.iter() {
            target_keys[index] = Some(key.clone());
        }
        let dependencies = module.call_dependencies();
        let mut reverse_dependencies = vec![BTreeSet::new(); dependencies.len()];
        for (caller, callees) in dependencies.iter().enumerate() {
            for &callee in callees {
                reverse_dependencies[callee].insert(caller);
            }
        }
        let environment = behavior_digest(&(
            "fluxr-structured-environment-v1",
            source.version,
            &source.inputs,
            &source.records,
            &source.enums,
        ))?;
        Ok(Self {
            behavior_cache: Arc::new(
                (0..source.functions.len())
                    .map(|_| Arc::new(OnceLock::new()))
                    .collect(),
            ),
            dependencies: Arc::new(dependencies),
            reverse_dependencies: Arc::new(reverse_dependencies),
            target_keys: Arc::new(target_keys),
            declaration_revisions: Arc::new(declaration_revisions),
            environment,
            source: Arc::new(source),
            module,
            targets,
            contracts,
            revisions: Arc::new(revisions),
            id,
        })
    }

    pub fn id(&self) -> Revision {
        self.id
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Current declaration value only. Use `behavior_revision` when a read also
    /// relies on callees; this legacy identity deliberately excludes them.
    pub fn revision(&self, target: &str) -> Option<Revision> {
        self.revisions.get(target).copied()
    }

    /// Pin a declaration and every statically reachable callee, including
    /// unregistered helpers and recursive/mutually recursive components.
    ///
    /// This v1 domain covers the admitted structured language: its source
    /// version, the full input/schema environment, and resolved call closure.
    /// It conservatively invalidates on any schema or input declaration change.
    /// Host input *values*, native implementation identity, and external host
    /// state are not represented; cache/test keys must bind those separately,
    /// along with compiler/engine producer identity. Changing this semantic
    /// interpretation requires a new behavior digest domain.
    /// This is an identity check, not a proof of behavioral equivalence.
    pub fn behavior_revision(&self, target: &str) -> Result<Option<Revision>, Error> {
        let Some(&(index, _)) = self.targets.get(target) else {
            return Ok(None);
        };
        self.behavior_cache[index]
            .get_or_init(|| {
                self.compute_behavior_revision(index)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map(|revision| Some(*revision))
            .map_err(|message| Error(message.clone()))
    }

    fn compute_behavior_revision(&self, index: usize) -> Result<Revision, Error> {
        let functions = self
            .dependency_closure(index)
            .into_iter()
            .map(|index| {
                let function = &self.source.functions[index];
                (
                    (function.receiver.as_deref(), function.name.as_str()),
                    function,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let function = &self.source.functions[index];
        behavior_digest(&(
            "fluxr-structured-behavior-v1",
            self.source.version,
            (&function.receiver, &function.name),
            &self.source.inputs,
            &self.source.records,
            &self.source.enums,
            // JSON object keys cannot be tuples; sorted entries retain both
            // the receiver and name without an ambiguous string delimiter.
            functions.into_iter().collect::<Vec<_>>(),
        ))
    }

    /// Build root pins from the compiler-resolved transitive call graph. Roots
    /// describe which behavior the author actually relied upon; this API cannot
    /// infer facts observed outside the supplied snapshot.
    pub fn behavior_read_set<'a>(
        &self,
        targets: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, Revision>, Error> {
        targets
            .into_iter()
            .map(|target| {
                self.behavior_revision(target)?
                    .map(|revision| (target.to_owned(), revision))
                    .ok_or_else(|| Error(format!("unknown behavioral target {target}")))
            })
            .collect()
    }

    fn dependency_closure(&self, root: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(index) = pending.pop() {
            if seen.insert(index) {
                pending.extend(self.dependencies[index].iter().copied());
            }
        }
        seen
    }

    /// Base pin for an edit. Reducer contributions pin the immutable contract,
    /// so they can arrive after other compatible contributions have activated.
    pub fn edit_revision(&self, target: &str) -> Option<Revision> {
        match self.targets.get(target)?.1 {
            Mode::Exclusive => self.revision(target),
            Mode::MinF32 => self.contracts.get(target).copied(),
        }
    }

    pub fn prepare(&self, changes: &[Change]) -> Result<Candidate, Error> {
        if changes.len() > MAX_CHANGES
            || changes
                .iter()
                .map(|change| change.edits.len())
                .sum::<usize>()
                > MAX_EDITS
        {
            return Err(Error("collaboration batch exceeds limits".into()));
        }
        let mut proposals = BTreeMap::new();
        let mut rejected = BTreeMap::new();
        for change in changes {
            let (id, bytes) = measured_digest(change)?;
            proposals.insert(id, change);
            if bound(bytes).is_err() {
                rejected.insert(
                    id,
                    Rejection::Invalid("proposal exceeds the source limit".into()),
                );
            }
        }
        self.finish_prepare(&proposals, rejected)
    }

    /// Prepare a deterministic same-base batch with behavioral read protection.
    /// A concurrent writer to any pinned root's reachable registered callee
    /// rejects the reader's whole change. Conflicts are conservative, including
    /// writes in unreachable-at-runtime branches. Self-contained edits in one
    /// change may update their own dependencies atomically.
    ///
    /// Accepted/rejected IDs hash the complete checked envelope in a distinct
    /// domain; they are intentionally not the legacy `prepare` proposal IDs.
    pub fn prepare_checked(&self, changes: &[CheckedChange]) -> Result<Candidate, Error> {
        if changes.len() > MAX_CHANGES
            || changes
                .iter()
                .map(|change| change.change.edits.len())
                .sum::<usize>()
                > MAX_EDITS
        {
            return Err(Error("collaboration batch exceeds limits".into()));
        }
        let mut expanded = BTreeMap::new();
        let mut rejected = BTreeMap::new();
        // Many agents can share the same behavioral context. Resolve and hash
        // each registered root once per immutable base/batch, then expand the
        // union of a change's dependencies only once.
        let mut pins = BTreeMap::new();
        for checked in changes {
            let (id, bytes) = measured_digest(&("fluxr-checked-change-v1", checked))?;
            if bound(bytes).is_err() {
                rejected.insert(
                    id,
                    Rejection::Invalid("proposal exceeds the source limit".into()),
                );
                // Retain its exact ID/rejection without making a second copy
                // of an oversized proposal just to discard it during compose.
                continue;
            }
            let mut change = checked.change.clone();
            {
                let mut reads = BTreeSet::new();
                for (target, expected) in &checked.behavior_requires {
                    let Some(&(index, _)) = self.targets.get(target) else {
                        rejected.insert(id, Rejection::Stale(target.clone()));
                        break;
                    };
                    if !pins.contains_key(target.as_str()) {
                        let actual = self
                            .behavior_revision(target)?
                            .ok_or_else(|| Error(format!("unknown behavioral target {target}")))?;
                        pins.insert(target.as_str(), (actual, self.dependency_closure(index)));
                    }
                    let (actual, closure) = &pins[target.as_str()];
                    if actual != expected {
                        rejected.insert(id, Rejection::Stale(target.clone()));
                        break;
                    }
                    reads.extend(closure.iter().copied());
                }
                if !rejected.contains_key(&id) {
                    for index in reads {
                        if let Some(dependency) = &self.target_keys[index] {
                            // Preserve a caller's explicit declaration pin,
                            // including an incorrect/stale one for validation.
                            change
                                .requires
                                .entry(dependency.clone())
                                .or_insert(self.revisions[dependency]);
                        }
                    }
                }
            }
            expanded.insert(id, change);
        }
        let proposals = expanded.iter().map(|(&id, change)| (id, change)).collect();
        self.finish_prepare(&proposals, rejected)
    }

    fn finish_prepare<'a>(
        &'a self,
        proposals: &BTreeMap<Revision, &'a Change>,
        mut rejected: BTreeMap<Revision, Rejection>,
    ) -> Result<Candidate, Error> {
        if proposals.keys().all(|id| rejected.contains_key(id)) {
            return Ok(Candidate {
                base: self.id,
                snapshot: self.clone(),
                accepted: vec![],
                rejected,
            });
        }
        let mut checker = BodyChecker::new(&self.source)?;
        for (&id, change) in proposals {
            if !rejected.contains_key(&id)
                && let Err(reason) = self.check(change, &mut checker)
            {
                rejected.insert(id, reason);
            }
        }
        let snapshot = self.compose(proposals, &mut rejected, &checker)?;
        let accepted = proposals
            .keys()
            .filter(|key| !rejected.contains_key(*key))
            .copied()
            .collect();
        Ok(Candidate {
            base: self.id,
            snapshot,
            accepted,
            rejected,
        })
    }

    fn check<'a>(
        &self,
        change: &'a Change,
        checker: &mut BodyChecker<'a>,
    ) -> Result<(), Rejection> {
        if change.author.is_empty() || change.edits.is_empty() {
            return Err(Rejection::Invalid("missing author or edit".into()));
        }
        for (target, &expected) in &change.requires {
            if self.revision(target) != Some(expected) {
                return Err(Rejection::Stale(target.clone()));
            }
        }
        let mut seen = BTreeSet::new();
        for edit in &change.edits {
            let target = edit.target();
            let Some(&(index, mode)) = self.targets.get(target) else {
                return Err(Rejection::Invalid(target.into()));
            };
            if !seen.insert(target) {
                return Err(Rejection::Invalid(target.into()));
            }
            if self.edit_revision(target) != Some(edit.expected()) {
                return Err(Rejection::Stale(target.into()));
            }
            match (mode, edit) {
                (Mode::Exclusive, Edit::ReplaceBody { body, .. }) => {
                    if self.changes_value(edit) {
                        checker
                            .check(index, body)
                            .map_err(|error| Rejection::Invalid(format!("{target}: {error}")))?;
                    }
                }
                (Mode::MinF32, Edit::MinF32 { bits, .. }) if valid_limit(*bits) => {}
                _ => return Err(Rejection::Invalid(target.into())),
            }
        }
        Ok(())
    }

    fn compose(
        &self,
        proposals: &BTreeMap<Revision, &Change>,
        rejected: &mut BTreeMap<Revision, Rejection>,
        checker: &BodyChecker<'_>,
    ) -> Result<Self, Error> {
        let mut bodies: BTreeMap<&str, BTreeMap<Revision, Vec<Revision>>> = BTreeMap::new();
        for (&id, change) in proposals {
            if rejected.contains_key(&id) {
                continue;
            }
            for edit in &change.edits {
                if let Edit::ReplaceBody { target, body, .. } = edit
                    && self.changes_value(edit)
                {
                    bodies
                        .entry(target)
                        .or_default()
                        .entry(digest(body)?)
                        .or_default()
                        .push(id);
                }
            }
        }
        for (target, alternatives) in bodies {
            if alternatives.len() > 1 {
                for id in alternatives.into_values().flatten() {
                    rejected
                        .entry(id)
                        .or_insert_with(|| Rejection::Conflict(target.into()));
                }
            }
        }

        let mut writers: BTreeMap<&str, BTreeSet<Revision>> = BTreeMap::new();
        for (&id, change) in proposals {
            if !rejected.contains_key(&id) {
                for edit in &change.edits {
                    if self.changes_value(edit) {
                        writers.entry(edit.target()).or_default().insert(id);
                    }
                }
            }
        }
        for (&id, change) in proposals {
            if rejected.contains_key(&id) {
                continue;
            }
            for target in change.requires.keys() {
                if writers
                    .get(target.as_str())
                    .is_some_and(|ids| ids.iter().any(|&other| other != id))
                {
                    rejected.insert(id, Rejection::DependencyChanged(target.clone()));
                    break;
                }
            }
        }
        self.apply(
            proposals
                .iter()
                .filter_map(|(id, &change)| (!rejected.contains_key(id)).then_some(change)),
            checker,
        )
    }

    fn changes_value(&self, edit: &Edit) -> bool {
        let (index, _) = self.targets[edit.target()];
        let current = &self.source.functions[index].body;
        match edit {
            Edit::ReplaceBody { body, .. } => body != current,
            Edit::MinF32 { bits, .. } => f32::from_bits(*bits) < literal_limit(current).unwrap(),
        }
    }

    fn apply<'a>(
        &self,
        changes: impl Iterator<Item = &'a Change>,
        checker: &BodyChecker<'_>,
    ) -> Result<Self, Error> {
        let mut edits = changes
            .flat_map(|change| &change.edits)
            .filter(|edit| self.changes_value(edit))
            .peekable();
        if edits.peek().is_none() {
            return Ok(self.clone());
        }
        let mut source = (*self.source).clone();
        let mut modified = BTreeSet::new();
        for edit in edits {
            let (index, _) = self.targets[edit.target()];
            modified.insert(index);
            let body = &mut source.functions[index].body;
            match edit {
                Edit::ReplaceBody { body: next, .. } => body.clone_from(next),
                Edit::MinF32 { bits, .. } => {
                    let limit = literal_limit(body).unwrap().min(f32::from_bits(*bits));
                    *body = vec![Stmt::Return {
                        value: Expr::F32 {
                            bits: limit.to_bits(),
                        },
                    }];
                }
            }
        }
        let module = Arc::new(self.module.replace_bodies(&source, checker, &modified)?);
        let mut revisions = (*self.revisions).clone();
        let mut declaration_revisions = (*self.declaration_revisions).clone();
        let mut dependencies = (*self.dependencies).clone();
        let mut reverse_dependencies = (*self.reverse_dependencies).clone();
        for &index in &modified {
            let revision = digest(&source.functions[index])?;
            declaration_revisions[index] = revision;
            let key = self.target_keys[index]
                .as_ref()
                .expect("registered edit target");
            revisions.insert(key.clone(), revision);
            let next = module.call_dependencies_at(index);
            for &callee in dependencies[index].difference(&next) {
                reverse_dependencies[callee].remove(&index);
            }
            for &callee in next.difference(&dependencies[index]) {
                reverse_dependencies[callee].insert(index);
            }
            dependencies[index] = next;
        }
        // A changed body may add or remove edges, but any unchanged root
        // that can reach that body is already in the old reverse closure.
        // Body replacement never changes the schema or declaration environment.
        let affected = self.reverse_closure(&modified);
        let behavior_cache = self
            .behavior_cache
            .iter()
            .enumerate()
            .map(|(index, memo)| {
                if affected.contains(&index) {
                    Arc::new(OnceLock::new())
                } else {
                    Arc::clone(memo)
                }
            })
            .collect();
        Ok(Self {
            id: snapshot_id(&module, &self.contracts)?,
            behavior_cache: Arc::new(behavior_cache),
            dependencies: Arc::new(dependencies),
            reverse_dependencies: Arc::new(reverse_dependencies),
            declaration_revisions: Arc::new(declaration_revisions),
            source: Arc::new(source),
            module,
            revisions: Arc::new(revisions),
            ..self.clone()
        })
    }
}

// Stream admitted source structures directly into a digest. A closure can be
// larger than the per-proposal bound, but is already bounded by module admission.
// Avoid constructing another full serialization of the reachable source.
fn behavior_digest(value: &impl Serialize) -> Result<Revision, Error> {
    measured_digest(value).map(|(revision, _)| revision)
}

// Hash the original serialization without allocating an equally large buffer.
// The byte count saturates so admission always rejects count overflow. This
// does not bound serialization CPU or caller-owned in-memory AST construction.
fn measured_digest(value: &impl Serialize) -> Result<(Revision, usize), Error> {
    struct HashWriter(Sha256, usize);
    impl std::io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            self.1 = self.1.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(Sha256::new(), 0);
    serde_json::to_writer(&mut writer, value).map_err(|error| Error(error.to_string()))?;
    Ok((Revision(writer.0.finalize().into()), writer.1))
}

fn snapshot_id(module: &Module, contracts: &BTreeMap<String, Revision>) -> Result<Revision, Error> {
    digest(&(
        Revision(Sha256::digest(module.canonical_source()).into()),
        contracts,
    ))
}

fn valid_limit(bits: u32) -> bool {
    let value = f32::from_bits(bits);
    value.is_finite() && !value.is_sign_negative()
}

fn literal_limit(body: &[Stmt]) -> Option<f32> {
    match body {
        [
            Stmt::Return {
                value: Expr::F32 { bits },
            },
        ] if valid_limit(*bits) => Some(f32::from_bits(*bits)),
        _ => None,
    }
}

pub struct Candidate {
    base: Revision,
    snapshot: Snapshot,
    pub accepted: Vec<Revision>,
    pub rejected: BTreeMap<Revision, Rejection>,
}

impl Candidate {
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Activation {
    pub tick: u64,
    pub before: Revision,
    pub after: Revision,
}

/// Owned by the host. The host calls activate between ticks, never during one.
pub struct LiveProgram {
    active: Snapshot,
}

impl LiveProgram {
    pub fn new(active: Snapshot) -> Self {
        Self { active }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.active
    }

    pub fn activate(&mut self, candidate: Candidate, tick: u64) -> Result<Activation, Error> {
        if candidate.base != self.active.id() {
            return Err(Error(
                "candidate was prepared against an older active snapshot".into(),
            ));
        }
        let activation = Activation {
            tick,
            before: self.active.id(),
            after: candidate.snapshot.id(),
        };
        self.active = candidate.snapshot;
        Ok(activation)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitCase {
    pub id: String,
    pub snapshot: Revision,
    pub export: String,
    #[serde(default)]
    pub inputs: Vec<Value>,
    #[serde(default)]
    pub arguments: Vec<Value>,
    pub limits: Limits,
    pub expected: Value,
}

impl UnitCase {
    pub fn run(&self, snapshot: &Snapshot, ceiling: Limits) -> Result<Revision, Error> {
        if self.snapshot != snapshot.id() {
            return Err(Error("unit case snapshot differs".into()));
        }
        if self.limits.instructions > ceiling.instructions
            || self.limits.cells > ceiling.cells
            || self.limits.frames > ceiling.frames
        {
            return Err(Error("unit case exceeds the host limits".into()));
        }
        let output = snapshot.module().run_with(
            Invocation {
                export: &self.export,
                inputs: &self.inputs,
                arguments: &self.arguments,
                limits: self.limits,
            },
            &mut Workspace::default(),
        )?;
        if output != self.expected {
            return Err(Error(format!("unit case {} returned {output:?}", self.id)));
        }
        digest(&(
            "fluxr-unit-v1",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self,
        ))
    }
}

/// A test fork has private host state and its own active program. The fixture's
/// Clone implementation must isolate mutable host effects.
pub struct TestRun<S> {
    state: S,
    program: LiveProgram,
}

impl<S: Clone> TestRun<S> {
    pub fn fork(state: &S, snapshot: &Snapshot) -> Self {
        Self {
            state: state.clone(),
            program: LiveProgram::new(snapshot.clone()),
        }
    }
}

impl<S> TestRun<S> {
    pub fn snapshot(&self) -> &Snapshot {
        self.program.snapshot()
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn activate(&mut self, candidate: Candidate, tick: u64) -> Result<Activation, Error> {
        self.program.activate(candidate, tick)
    }

    pub fn step<T>(&mut self, step: impl FnOnce(&mut S, &Snapshot) -> T) -> T {
        step(&mut self.state, self.program.snapshot())
    }
}
