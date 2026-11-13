//! Versioned, deterministic modules with typed state and bounded execution.
mod agent;
pub mod authoring;
mod compact;
mod compact_print;
mod compile;
#[cfg(all(
    feature = "typed-jit",
    target_os = "linux",
    target_env = "gnu",
    target_arch = "x86_64"
))]
mod linux_jit;
mod native_aot;
mod native_budget;
mod native_ir;
#[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
mod native_scalar;
#[cfg(all(
    feature = "typed-jit",
    target_arch = "x86_64",
    any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
))]
mod typed_jit;
mod unified_source;
mod vm;
pub mod wordcode;

use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) use compact::parse_expression;
pub(crate) use compile::BodyChecker;
pub use compile::Module;
pub use native_aot::{NativeKernel, OwnedPreparedAot, PreparedAot};
pub use unified_source::{NativeItemSpan, SourceDocument};
pub use vm::{ExecutionRoute, PreparedCall, Workspace};

const VERSION: u16 = 2;
const MAGIC: &[u8; 4] = b"FSM\0";
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub version: u16,
    pub inputs: Vec<Binding>,
    pub records: Vec<Record>,
    pub enums: Vec<Enum>,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    Unit,
    Bool,
    I64,
    U64,
    F32,
    Record(String),
    Enum(String),
    List(Box<Type>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub name: String,
    pub fields: Vec<Binding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enum {
    pub name: String,
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    pub payload: Option<Type>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Function {
    pub name: String,
    #[serde(default)]
    pub receiver: Option<String>,
    #[serde(default)]
    pub export: bool,
    #[serde(default)]
    pub params: Vec<Binding>,
    #[serde(default)]
    pub locals: Vec<Binding>,
    pub result: Type,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stmt {
    Set {
        local: String,
        value: Expr,
    },
    SetField {
        local: String,
        field: String,
        value: Expr,
    },
    Push {
        local: String,
        value: Expr,
    },
    Pop {
        local: String,
    },
    If {
        test: Expr,
        then: Vec<Stmt>,
        #[serde(default)]
        otherwise: Vec<Stmt>,
    },
    While {
        test: Expr,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Return {
        value: Expr,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Unit,
    Bool {
        value: bool,
    },
    I64 {
        value: i64,
    },
    U64 {
        value: u64,
    },
    /// An IEEE-754 binary32 literal written as its bit pattern.
    F32 {
        bits: u32,
    },
    Local {
        name: String,
    },
    Input {
        name: String,
    },
    Binary {
        kind: Binary,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        kind: Unary,
        value: Box<Expr>,
    },
    Record {
        name: String,
        fields: Vec<Expr>,
    },
    Field {
        record: Box<Expr>,
        name: String,
    },
    Variant {
        name: String,
        variant: String,
        payload: Option<Box<Expr>>,
    },
    IsVariant {
        value: Box<Expr>,
        variant: String,
    },
    Payload {
        value: Box<Expr>,
        variant: String,
    },
    List {
        element: Type,
        #[serde(default)]
        items: Vec<Expr>,
    },
    Empty {
        list: Box<Expr>,
    },
    Last {
        list: Box<Expr>,
    },
    Index {
        list: Box<Expr>,
        index: Box<Expr>,
    },
    ArgMinF32 {
        list: Box<Expr>,
    },
    Call {
        name: String,
        #[serde(default)]
        args: Vec<Expr>,
    },
    /// A typed, explicitly registered native service. The runtime must admit
    /// the exact signature before an injected module can be published.
    HostCall {
        name: String,
        params: Vec<Type>,
        result: Type,
        args: Vec<Expr>,
    },
    Method {
        receiver: Box<Expr>,
        name: String,
        #[serde(default)]
        args: Vec<Expr>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Lt,
    Le,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unary {
    Not,
    AbsF32,
    U64ToF32,
    I64ToF32,
    F32ToI64,
    SqrtF32,
}

/// A host-owned value. Records and enum tags follow their declared field order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    F32(u32),
    Record(Arc<[Value]>),
    Variant(usize, Option<Arc<Value>>),
    List(Arc<Vec<Value>>),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub instructions: u64,
    /// Aggregate cells visited at the boundary or allocated during a call.
    pub cells: usize,
    pub frames: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            instructions: 50_000,
            cells: 16_384,
            frames: 64,
        }
    }
}

/// Inputs are a read-only host snapshot in declared order. Arguments, results
/// and persistent state remain owned by the host.
pub struct Invocation<'a> {
    pub export: &'a str,
    pub inputs: &'a [Value],
    pub arguments: &'a [Value],
    pub limits: Limits,
}

/// Native services are a capability boundary, not access to arbitrary Rust
/// memory. Implementations must respect their own I/O and cancellation bounds.
pub trait HostServices: Send + Sync {
    fn call(&self, name: &str, arguments: &[Value]) -> Result<Value, Error>;
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostSignature {
    pub params: Vec<Type>,
    pub result: Type,
}

impl Module {
    pub fn same_interface(&self, other: &Self) -> bool {
        self.interface == other.interface
    }

    pub fn input_bindings(&self) -> &[Binding] {
        &self.interface.inputs
    }

    pub fn export_signatures(&self) -> &BTreeMap<String, (Vec<Type>, Type)> {
        &self.interface.exports
    }

    /// Return all native service requirements, including unreachable bodies.
    /// One service name cannot be assigned inconsistent signatures.
    pub fn host_requirements(&self) -> Result<BTreeMap<String, HostSignature>, Error> {
        let mut result = BTreeMap::new();
        for code in &self.code {
            for instruction in &code.ops {
                if let compile::Op::HostCall { name, signature } = &instruction.op {
                    if let Some(prior) = result.insert(name.clone(), signature.clone()) {
                        if prior != *signature {
                            return Err(error(format!("inconsistent host signature for {name}")));
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    /// Check the transitive call graph, so an effectful export cannot enter a
    /// native tier whose speculative fallback would repeat host effects.
    pub fn export_uses_host(&self, export: &str) -> bool {
        let Some(&entry) = self.exports.get(export) else {
            return false;
        };
        let mut pending = vec![entry];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            for instruction in &self.code[index].ops {
                match instruction.op {
                    compile::Op::HostCall { .. } => return true,
                    compile::Op::Call(next) => pending.push(next),
                    _ => {}
                }
            }
        }
        false
    }

    pub fn export_host_requirements(
        &self,
        export: &str,
    ) -> Result<BTreeMap<String, HostSignature>, Error> {
        let &entry = self
            .exports
            .get(export)
            .ok_or_else(|| error(format!("unknown export {export}")))?;
        let mut pending = vec![entry];
        let mut seen = std::collections::BTreeSet::new();
        let mut requirements = BTreeMap::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            for instruction in &self.code[index].ops {
                match &instruction.op {
                    compile::Op::HostCall { name, signature } => {
                        requirements.insert(name.clone(), signature.clone());
                    }
                    compile::Op::Call(next) => pending.push(*next),
                    _ => {}
                }
            }
        }
        Ok(requirements)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Interface {
    inputs: Vec<Binding>,
    exports: BTreeMap<String, (Vec<Type>, Type)>,
    records: BTreeMap<String, Record>,
    enums: BTreeMap<String, Enum>,
}

/// An explicitly replaced module at a host-controlled transaction boundary.
pub struct ModuleSlot {
    active: Module,
    revision: u64,
}

impl ModuleSlot {
    /// Grants list the snapshot inputs the trusted host authorizes.
    pub fn new(active: Module, grants: &[Binding]) -> Result<Self, Error> {
        for required in &active.interface.inputs {
            if !grants.contains(required) {
                return Err(error(format!(
                    "host input {} is not granted",
                    required.name
                )));
            }
        }
        Ok(Self {
            active,
            revision: 0,
        })
    }

    pub fn module(&self) -> &Module {
        &self.active
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn run(&self, invocation: Invocation<'_>) -> Result<Value, Error> {
        self.active.run(invocation)
    }

    pub fn run_with(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        self.active.run_with(invocation, workspace)
    }

    /// Verify the same host interface, and optionally evaluate the candidate
    /// on a read-only snapshot, before publishing a new revision.
    pub fn inject(
        &mut self,
        expected_revision: u64,
        candidate: Module,
        probe: Option<Invocation<'_>>,
    ) -> Result<u64, Error> {
        if self.revision != expected_revision {
            return Err(error("module revision changed"));
        }
        if candidate.interface != self.active.interface {
            return Err(error("replacement module has a different host interface"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| error("module revision overflow"))?;
        if let Some(probe) = probe {
            candidate.run(probe)?;
        }
        self.active = candidate;
        self.revision = revision;
        Ok(revision)
    }
}

pub(crate) fn error(message: impl Into<String>) -> Error {
    Error(message.into())
}
