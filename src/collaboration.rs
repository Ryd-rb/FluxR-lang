//! Immutable authoring snapshots for structured FluxR modules.

use crate::structured::{
    BodyChecker, Expr, Invocation, Limits, Module, Source, Stmt, Type, Value, Workspace,
};
use crate::{Error, bound};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
    let bytes = serde_json::to_vec(value).map_err(|e| Error(e.to_string()))?;
    bound(bytes.len())?;
    Ok(Revision(Sha256::digest(bytes).into()))
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
    id: Revision,
}

impl Snapshot {
    /// Bind stable IDs once. Function order and spellings in existing source
    /// remain compatible with the structured module format.
    pub fn new(source: Source, targets: Vec<Target>) -> Result<Self, Error> {
        let mut slots = BTreeMap::new();
        let mut contracts = BTreeMap::new();
        let mut registered = BTreeSet::new();
        for target in targets {
            let index = source
                .functions
                .iter()
                .position(|f| f.name == target.function && f.receiver == target.receiver)
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
        Self::build(source, Arc::new(slots), Arc::new(contracts))
    }

    fn build(
        source: Source,
        targets: Arc<BTreeMap<String, (usize, Mode)>>,
        contracts: Arc<BTreeMap<String, Revision>>,
    ) -> Result<Self, Error> {
        let module = Arc::new(Module::compile(source.clone())?);
        let id = snapshot_id(&module, &contracts)?;
        let revisions = targets
            .iter()
            .map(|(key, &(index, _))| Ok((key.clone(), digest(&source.functions[index])?)))
            .collect::<Result<_, Error>>()?;
        Ok(Self {
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

    /// Current declaration value, for reads that rely on exact behavior.
    pub fn revision(&self, target: &str) -> Option<Revision> {
        self.revisions.get(target).copied()
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
            let bytes = serde_json::to_vec(change).map_err(|e| Error(e.to_string()))?;
            let id = Revision(Sha256::digest(&bytes).into());
            proposals.insert(id, change);
            if bound(bytes.len()).is_err() {
                rejected.insert(
                    id,
                    Rejection::Invalid("proposal exceeds the source limit".into()),
                );
            }
        }
        let checker = BodyChecker::new(&self.source)?;
        for (&id, change) in &proposals {
            if !rejected.contains_key(&id)
                && let Err(reason) = self.check(change, &checker)
            {
                rejected.insert(id, reason);
            }
        }
        let snapshot = self.compose(&proposals, &mut rejected, &checker)?;
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

    fn check(&self, change: &Change, checker: &BodyChecker<'_>) -> Result<(), Rejection> {
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
        let mut source = (*self.source).clone();
        let mut modified = BTreeSet::new();
        for edit in changes.flat_map(|change| &change.edits) {
            if !self.changes_value(edit) {
                continue;
            }
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
        if modified.is_empty() {
            return Ok(self.clone());
        }
        let module = Arc::new(self.module.replace_bodies(&source, checker, &modified)?);
        let mut revisions = (*self.revisions).clone();
        for (key, &(index, _)) in self.targets.iter() {
            if modified.contains(&index) {
                revisions.insert(key.clone(), digest(&source.functions[index])?);
            }
        }
        Ok(Self {
            id: snapshot_id(&module, &self.contracts)?,
            source: Arc::new(source),
            module,
            revisions: Arc::new(revisions),
            ..self.clone()
        })
    }
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
