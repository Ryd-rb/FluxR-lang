//! Checked migration IR. The origin determines the ABI, traps, and metering
//! when the IR is restored to a released artifact.

use crate::rules::{Binary as RuleBinary, Op};
use crate::structured::{
    Binary, Binding, Expr, Function, Invocation, Limits, Module, Source, Stmt, Type, Value,
    Workspace,
};
use crate::{Error, RuleProgram, bound};

const MAGIC: &[u8; 4] = b"FXH\0";
const VERSION: u8 = 1;
const STRUCTURED: u8 = 0;
const RULE: u8 = 1;
const HEADER_SIZE: usize = 10;

/// The released executable artifact and its original host contract.
pub enum Artifact {
    Structured(Module),
    Rule(RuleProgram),
}

/// A validated FXH program with enough information to restore its origin.
pub struct Hir(Body);

enum Body {
    Structured(Module),
    Rule {
        origin: RuleProgram,
        projection: Result<RuleProjection, &'static str>,
    },
}

struct RuleProjection {
    module: Module,
    compact: String,
}

/// An opt-in, in-memory rule projection with the structured VM's limits.
/// Restore the original FXD for the released rule ABI and metering.
pub struct CheckedRuleProjection(Module);

impl CheckedRuleProjection {
    pub fn run_with(
        &self,
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        self.0.run_portable_with(
            Invocation {
                export: "rule",
                inputs: &[],
                arguments,
                limits,
            },
            workspace,
        )
    }
}

impl Hir {
    pub fn from_structured(module: Module) -> Self {
        Self(Body::Structured(module))
    }

    pub fn from_rule(rule: RuleProgram) -> Result<Self, Error> {
        let projection = match project_rule(&rule, false) {
            Ok(source) => {
                let module = Module::compile_borrowed(&source)?;
                let compact = source.checked_compact_text()?;
                Ok(RuleProjection { module, compact })
            }
            Err(reason) => Err(reason),
        };
        Ok(Self(Body::Rule {
            origin: rule,
            projection,
        }))
    }

    /// Inspect the shared structured source. A rule projection represents its
    /// computation, while the rule origin defines its host contract.
    pub fn source(&self) -> Result<Source, Error> {
        let module = match &self.0 {
            Body::Structured(module)
            | Body::Rule {
                projection: Ok(RuleProjection { module, .. }),
                ..
            } => module,
            Body::Rule {
                projection: Err(reason),
                ..
            } => return Err(Error((*reason).into())),
        };
        module.checked_source()
    }

    /// Prepare a rule for checked structured execution, including signed
    /// bitwise and and raw binary32. This projection is separate from FXH v1.
    pub fn prepare_rule_projection(&self) -> Result<CheckedRuleProjection, Error> {
        let Body::Rule { origin, projection } = &self.0 else {
            return Err(Error(
                "checked rule projection requires a rule origin".into(),
            ));
        };
        if let Ok(projection) = projection {
            return Ok(CheckedRuleProjection(projection.module.clone()));
        }
        let source = project_rule(origin, true).map_err(|reason| Error(reason.into()))?;
        Ok(CheckedRuleProjection(Module::compile_rule_projection(
            &source,
        )?))
    }

    pub fn into_artifact(self) -> Artifact {
        match self.0 {
            Body::Structured(module) => Artifact::Structured(module),
            Body::Rule { origin, .. } => Artifact::Rule(origin),
        }
    }

    /// Write canonical compact HIR. Rules also retain their checked FXD
    /// origin, which records the exact version and the rule-only operations.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend([VERSION, STRUCTURED]);
        bytes.extend([0; 4]);
        match &self.0 {
            Body::Structured(module) => {
                bytes.extend_from_slice(module.compact_text()?.as_bytes());
            }
            Body::Rule { origin, projection } => {
                bytes[5] = RULE;
                let origin = origin.encode();
                let size = u16::try_from(origin.len())
                    .map_err(|_| Error("FXH rule origin exceeds length limit".into()))?;
                bytes.extend_from_slice(&size.to_le_bytes());
                bytes.extend_from_slice(&origin);
                if let Ok(projection) = projection {
                    bytes.extend_from_slice(projection.compact.as_bytes());
                }
            }
        }
        let size = u32::try_from(bytes.len() - HEADER_SIZE)
            .map_err(|_| Error("FXH payload exceeds length limit".into()))?;
        bytes[6..HEADER_SIZE].copy_from_slice(&size.to_le_bytes());
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let ([version, kind, size @ ..], payload) = bytes
            .strip_prefix(MAGIC)
            .and_then(|rest| rest.split_first_chunk::<6>())
            .ok_or_else(|| Error("invalid FXH envelope".into()))?;
        if *version != VERSION {
            return Err(Error(format!("unsupported FXH version {version}")));
        }
        if u32::from_le_bytes(*size) as usize != payload.len() {
            return Err(Error("FXH payload size differs from envelope".into()));
        }
        let hir = match *kind {
            STRUCTURED => {
                bound(payload.len())?;
                let source =
                    std::str::from_utf8(payload).map_err(|error| Error(error.to_string()))?;
                Self::from_structured(Module::compile_compact(source)?)
            }
            RULE => {
                let (size, data) = payload
                    .split_first_chunk::<2>()
                    .ok_or_else(|| Error("missing FXH rule origin size".into()))?;
                let (origin, projection) = data
                    .split_at_checked(usize::from(u16::from_le_bytes(*size)))
                    .ok_or_else(|| Error("truncated FXH rule origin".into()))?;
                bound(projection.len())?;
                Self::from_rule(RuleProgram::decode(origin)?)?
            }
            _ => return Err(Error(format!("unsupported FXH kind {kind}"))),
        };
        if hir.encode()? != bytes {
            return Err(Error("noncanonical FXH payload".into()));
        }
        Ok(hir)
    }
}

fn project_rule(rule: &RuleProgram, rule_only_ops: bool) -> Result<Source, &'static str> {
    let (inputs, ops) = rule.hir_parts();
    let mut depth = 0;
    let mut slots = 0;
    let mut body = Vec::with_capacity(ops.len() + 1);
    for op in ops {
        match *op {
            Op::Input(index) => {
                body.push(store(depth, local(format!("a{index}"))));
                depth += 1;
            }
            Op::Const(value) => {
                body.push(store(depth, Expr::I64 { value }));
                depth += 1;
            }
            Op::Binary(kind) => {
                depth -= 1;
                body.push(project_binary(kind, depth - 1, depth, rule_only_ops)?);
            }
            Op::Select => {
                depth -= 2;
                let destination = depth - 1;
                body.push(choose(
                    binary(Binary::Eq, slot(destination), Expr::I64 { value: 0 }),
                    store(destination, slot(depth + 1)),
                    store(destination, slot(depth)),
                ));
            }
            Op::Guard => {
                depth -= 1;
                body.push(Stmt::If {
                    test: binary(Binary::Eq, slot(depth), Expr::I64 { value: 0 }),
                    then: vec![Stmt::Return {
                        value: Expr::I64 { value: 0 },
                    }],
                    otherwise: vec![],
                });
            }
        }
        slots = slots.max(depth);
    }
    body.push(Stmt::Return { value: slot(0) });
    let binding = |name| Binding {
        name,
        ty: Type::I64,
    };
    Ok(Source {
        version: 1,
        inputs: vec![],
        records: vec![],
        enums: vec![],
        functions: vec![Function {
            name: "rule".into(),
            receiver: None,
            export: true,
            params: (0..inputs)
                .map(|index| binding(format!("a{index}")))
                .collect(),
            locals: (0..slots)
                .map(|index| binding(format!("s{index}")))
                .collect(),
            result: Type::I64,
            body,
        }],
    })
}

fn project_binary(
    kind: RuleBinary,
    left: usize,
    right: usize,
    rule_only_ops: bool,
) -> Result<Stmt, &'static str> {
    let operation = match kind {
        RuleBinary::Add => Binary::Add,
        RuleBinary::Sub => Binary::Sub,
        RuleBinary::Mul => Binary::Mul,
        RuleBinary::Div => Binary::Div,
        RuleBinary::Rem => Binary::Rem,
        RuleBinary::Eq => Binary::Eq,
        RuleBinary::Lt | RuleBinary::Min | RuleBinary::Max => Binary::Lt,
        RuleBinary::BitAnd if rule_only_ops => Binary::And,
        RuleBinary::BitAnd => return Err("structured projection lacks rule integer bitwise and"),
        RuleBinary::F32Lt
        | RuleBinary::F32Le
        | RuleBinary::F32Add
        | RuleBinary::F32Sub
        | RuleBinary::F32Mul
        | RuleBinary::F32Div => {
            if !rule_only_ops {
                return Err("structured projection lacks rule raw binary32 transport");
            }
            return Ok(store(
                left,
                Expr::Call {
                    name: kind.token().into(),
                    args: vec![slot(left), slot(right)],
                },
            ));
        }
    };
    let expression = binary(operation, slot(left), slot(right));
    Ok(match kind {
        RuleBinary::Eq | RuleBinary::Lt => choose(
            expression,
            store(left, Expr::I64 { value: 1 }),
            store(left, Expr::I64 { value: 0 }),
        ),
        RuleBinary::Min => choose(
            expression,
            store(left, slot(left)),
            store(left, slot(right)),
        ),
        RuleBinary::Max => choose(
            expression,
            store(left, slot(right)),
            store(left, slot(left)),
        ),
        _ => store(left, expression),
    })
}

fn local(name: String) -> Expr {
    Expr::Local { name }
}

fn slot(index: usize) -> Expr {
    local(format!("s{index}"))
}

fn store(index: usize, value: Expr) -> Stmt {
    Stmt::Set {
        local: format!("s{index}"),
        value,
    }
}

fn binary(kind: Binary, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn choose(test: Expr, then: Stmt, otherwise: Stmt) -> Stmt {
    Stmt::If {
        test,
        then: vec![then],
        otherwise: vec![otherwise],
    }
}
