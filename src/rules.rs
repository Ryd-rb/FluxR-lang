//! Bounded rules for deterministic host decisions.
use crate::telemetry::{Feature, Work, measured};
use crate::{Error, bound, number};
use std::fmt::Write;

#[path = "rules_codegen.rs"]
mod codegen;
#[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
#[path = "native_jit.rs"]
pub(crate) mod native_jit;

const MAGIC: &[u8; 4] = b"FXD\0";
const MAX_VERSION: u8 = 4;
const CANONICAL_F32_NAN: u32 = 0x7fc0_0000;
pub(crate) const MAX_INPUTS: u8 = 16;
const MAX_OPS: usize = 256;
const MAX_STACK: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Input(u8),
    Const(i64),
    Binary(Binary),
    Select,
    Guard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Binary {
    Add = 2,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Eq,
    Min,
    Max,
    BitAnd = 12,
    F32Lt,
    F32Le,
    F32Add = 16,
    F32Sub,
    F32Mul,
    F32Div,
}

impl Binary {
    const ALL: [Self; 16] = [
        Self::Add,
        Self::Sub,
        Self::Mul,
        Self::Div,
        Self::Rem,
        Self::Lt,
        Self::Eq,
        Self::Min,
        Self::Max,
        Self::BitAnd,
        Self::F32Lt,
        Self::F32Le,
        Self::F32Add,
        Self::F32Sub,
        Self::F32Mul,
        Self::F32Div,
    ];

    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Lt => "<",
            Self::Eq => "=",
            Self::Min => "m",
            Self::Max => "M",
            Self::BitAnd => "&",
            Self::F32Lt => "f<",
            Self::F32Le => "f<=",
            Self::F32Add => "f+",
            Self::F32Sub => "f-",
            Self::F32Mul => "f*",
            Self::F32Div => "f/",
        }
    }

    pub(crate) fn raw_f32_token(token: &str) -> Option<Self> {
        if !token.starts_with('f') {
            return None;
        }
        Self::ALL.into_iter().find(|op| op.token() == token)
    }

    pub(crate) fn evaluate(self, left: i64, right: i64) -> Option<i64> {
        match self {
            Self::Add => left.checked_add(right),
            Self::Sub => left.checked_sub(right),
            Self::Mul => left.checked_mul(right),
            Self::Div => left.checked_div(right),
            Self::Rem => left.checked_rem(right),
            Self::Lt => Some(i64::from(left < right)),
            Self::Eq => Some(i64::from(left == right)),
            Self::Min => Some(left.min(right)),
            Self::Max => Some(left.max(right)),
            Self::BitAnd => Some(left & right),
            Self::F32Lt => Some(i64::from(
                f32::from_bits(left as u32) < f32::from_bits(right as u32),
            )),
            Self::F32Le => Some(i64::from(
                f32::from_bits(left as u32) <= f32::from_bits(right as u32),
            )),
            Self::F32Add | Self::F32Sub | Self::F32Mul | Self::F32Div => {
                let left = f32::from_bits(left as u32);
                let right = f32::from_bits(right as u32);
                let result = match self {
                    Self::F32Add => left + right,
                    Self::F32Sub => left - right,
                    Self::F32Mul => left * right,
                    Self::F32Div => left / right,
                    _ => unreachable!(),
                };
                Some(i64::from(if result.is_nan() {
                    CANONICAL_F32_NAN
                } else {
                    result.to_bits()
                }))
            }
        }
    }
}

impl Op {
    fn code(self) -> u8 {
        match self {
            Self::Input(_) => 0,
            Self::Const(_) => 1,
            Self::Binary(operation) => operation as u8,
            Self::Select => 11,
            Self::Guard => 15,
        }
    }

    fn from_code(code: u8, operand: Option<i64>) -> Result<Self, Error> {
        match (code, operand) {
            (0, Some(index)) => Ok(Self::Input(u8::try_from(index).map_err(|_| invalid_op())?)),
            (1, Some(value)) => Ok(Self::Const(value)),
            (11, None) => Ok(Self::Select),
            (code @ 2..=10, None) => Ok(Self::Binary(Binary::ALL[usize::from(code - 2)])),
            (code @ 12..=14, None) => Ok(Self::Binary(Binary::ALL[usize::from(code - 3)])),
            (15, None) => Ok(Self::Guard),
            (code @ 16..=19, None) => Ok(Self::Binary(Binary::ALL[usize::from(code - 4)])),
            _ => Err(invalid_op()),
        }
    }

    fn parse(token: &str, version: u8) -> Result<Self, Error> {
        if let Some(index) = token.strip_prefix('i') {
            return Ok(Self::Input(number(Some(index), "input index")?));
        }
        if token == "?" {
            return Ok(Self::Select);
        }
        if token == "guard" {
            return Ok(Self::Guard);
        }
        if let Some(operation) = Binary::ALL.into_iter().find(|op| op.token() == token) {
            return Ok(Self::Binary(operation));
        }
        if version >= 3
            && let Some(literal) = token.strip_prefix("f32:")
        {
            let value: f32 = number(Some(literal), "f32 rule operand")?;
            return Ok(Self::Const(i64::from(value.to_bits())));
        }
        Ok(Self::Const(number(Some(token), "rule operand")?))
    }
}

fn invalid_op() -> Error {
    Error("invalid rule opcode or operand".into())
}

/// Portable, side-effect-free postfix program with a single i64 result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleProgram {
    version: u8,
    inputs: u8,
    ops: Vec<Op>,
}

/// An evaluated rule with its portable representation retained. Native
/// compilation is optional and an unavailable backend leaves the VM active.
pub struct PreparedRule {
    program: RuleProgram,
    #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
    native: Option<native_jit::NativeRule>,
}

impl PreparedRule {
    pub fn program(&self) -> &RuleProgram {
        &self.program
    }

    pub fn uses_native_code(&self) -> bool {
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        return self.native.is_some();
        #[cfg(not(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64")))]
        false
    }

    #[inline]
    pub fn evaluate(&self, inputs: &[i64]) -> Result<i64, Error> {
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        if let Some(native) = &self.native {
            let result = native.evaluate(inputs);
            crate::telemetry::record_rule_evaluation(result.is_ok());
            return result;
        }
        self.program.evaluate(inputs)
    }
}

impl RuleProgram {
    pub(crate) fn hir_parts(&self) -> (u8, &[Op]) {
        (self.inputs, &self.ops)
    }

    pub fn prepare(self) -> PreparedRule {
        PreparedRule {
            #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
            native: native_jit::compile(&self).ok(),
            program: self,
        }
    }

    /// Compile `r<version> <input-count>` followed by whitespace-separated operations.
    pub fn compile_text(source: &str) -> Result<Self, Error> {
        measured(
            Feature::CompileText,
            source.len(),
            || Self::parse_text(source),
            |_| Work::default(),
        )
    }

    fn parse_text(source: &str) -> Result<Self, Error> {
        bound(source.len())?;
        let version = number(
            source
                .split_whitespace()
                .next()
                .and_then(|word| word.strip_prefix('r')),
            "rule version",
        )?;
        let mut tokens = source.lines().flat_map(|line| {
            let code = if version >= 4 {
                line.split_once('#').map_or(line, |(code, _)| code)
            } else {
                line
            };
            code.split_whitespace()
        });
        tokens.next();
        let inputs = number(tokens.next(), "rule input count")?;
        let ops = tokens
            .take(MAX_OPS + 1)
            .map(|token| Op::parse(token, version))
            .collect::<Result<_, _>>()?;
        Self::compile(version, inputs, ops)
    }

    /// Compile `[version,input-count,[[opcode,operand?],...]]`.
    pub fn compile_structured(source: &str) -> Result<Self, Error> {
        measured(
            Feature::CompileTyped,
            source.len(),
            || Self::parse_structured(source),
            |_| Work::default(),
        )
    }

    fn parse_structured(source: &str) -> Result<Self, Error> {
        bound(source.len())?;
        let (version, inputs, rows): (u8, u8, Vec<Vec<i64>>) =
            serde_json::from_str(source).map_err(|error| Error(error.to_string()))?;
        if rows.len() > MAX_OPS {
            return Err(Error("rule instruction count outside limits".into()));
        }
        let ops = rows
            .iter()
            .map(|row| {
                let (code, operand) = match row.as_slice() {
                    [code] => (*code, None),
                    [code, operand] => (*code, Some(*operand)),
                    _ => return Err(invalid_op()),
                };
                Op::from_code(u8::try_from(code).map_err(|_| invalid_op())?, operand)
            })
            .collect::<Result<_, _>>()?;
        Self::compile(version, inputs, ops)
    }

    pub(crate) fn compile(version: u8, inputs: u8, ops: Vec<Op>) -> Result<Self, Error> {
        if !(1..=MAX_VERSION).contains(&version) {
            return Err(Error(format!("unsupported rule version {version}")));
        }
        if inputs > MAX_INPUTS || !(1..=MAX_OPS).contains(&ops.len()) {
            return Err(Error(
                "rule inputs or instruction count outside limits".into(),
            ));
        }
        let mut depth = 0;
        for &operation in &ops {
            let consumed = match operation {
                Op::Binary(Binary::BitAnd) if version < 2 => {
                    return Err(Error("bitwise and requires rule version 2".into()));
                }
                Op::Binary(Binary::F32Lt | Binary::F32Le) if version < 3 => {
                    return Err(Error("f32 comparison requires rule version 3".into()));
                }
                Op::Binary(Binary::F32Add | Binary::F32Sub | Binary::F32Mul | Binary::F32Div)
                    if version < 4 =>
                {
                    return Err(Error("f32 arithmetic requires rule version 4".into()));
                }
                Op::Guard if version < 3 => {
                    return Err(Error("guard requires rule version 3".into()));
                }
                Op::Input(index) if index >= inputs => {
                    return Err(Error("unknown rule input".into()));
                }
                Op::Input(_) | Op::Const(_) => 0,
                Op::Binary(_) => 2,
                Op::Select => 3,
                Op::Guard => 1,
            };
            if depth < consumed || depth - consumed >= MAX_STACK {
                return Err(Error("rule stack outside limits".into()));
            }
            depth = depth - consumed + usize::from(operation != Op::Guard);
        }
        if depth != 1 {
            return Err(Error("rule must produce one result".into()));
        }
        Ok(Self {
            version,
            inputs,
            ops,
        })
    }

    pub fn text(&self) -> String {
        let mut result = format!("r{} {}\n", self.version, self.inputs);
        for (index, op) in self.ops.iter().enumerate() {
            if index != 0 {
                result.push(' ');
            }
            match op {
                Op::Input(input) => write!(result, "i{input}").unwrap(),
                Op::Const(value) => write!(result, "{value}").unwrap(),
                Op::Binary(operation) => result.push_str(operation.token()),
                Op::Select => result.push('?'),
                Op::Guard => result.push_str("guard"),
            }
        }
        result
    }

    pub fn structured(&self) -> String {
        let ops: Vec<Vec<i64>> = self
            .ops
            .iter()
            .map(|op| {
                let code = i64::from(op.code());
                match *op {
                    Op::Input(index) => vec![code, i64::from(index)],
                    Op::Const(value) => vec![code, value],
                    _ => vec![code],
                }
            })
            .collect();
        serde_json::to_string(&(self.version, self.inputs, ops)).unwrap()
    }

    pub fn encode(&self) -> Vec<u8> {
        let attempt = crate::telemetry::Attempt::start(Feature::Encode, 0);
        let mut bytes = Vec::from(MAGIC);
        bytes.extend([self.version, self.inputs]);
        bytes.extend((self.ops.len() as u16).to_le_bytes());
        for &op in &self.ops {
            bytes.push(op.code());
            match op {
                Op::Input(index) => bytes.push(index),
                Op::Const(value) => bytes.extend(value.to_le_bytes()),
                _ => (),
            }
        }
        if let Some(attempt) = attempt {
            attempt.finish_output(bytes.len());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        measured(
            Feature::Decode,
            bytes.len(),
            || Self::read_binary(bytes),
            |_| Work::default(),
        )
    }

    fn read_binary(bytes: &[u8]) -> Result<Self, Error> {
        let mut rest = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| Error("invalid rule magic".into()))?;
        let [version, inputs] = read(&mut rest)?;
        let count = usize::from(u16::from_le_bytes(read(&mut rest)?));
        if !(1..=MAX_OPS).contains(&count) {
            return Err(Error("rule instruction count outside limits".into()));
        }
        let mut ops = Vec::with_capacity(count);
        for _ in 0..count {
            let [code] = read(&mut rest)?;
            let operand = match code {
                0 => Some(i64::from(read::<1>(&mut rest)?[0])),
                1 => Some(i64::from_le_bytes(read(&mut rest)?)),
                _ => None,
            };
            ops.push(Op::from_code(code, operand)?);
        }
        if !rest.is_empty() {
            return Err(Error("trailing bytes in rule".into()));
        }
        Self::compile(version, inputs, ops)
    }

    /// Arithmetic errors are checked; select eagerly chooses from two computed
    /// values, treating zero as false. Evaluation does not allocate on success.
    pub fn evaluate(&self, inputs: &[i64]) -> Result<i64, Error> {
        let outcome = self.run(inputs);
        crate::telemetry::record_rule_evaluation(outcome.is_ok());
        outcome
    }

    fn run(&self, inputs: &[i64]) -> Result<i64, Error> {
        if inputs.len() != usize::from(self.inputs) {
            return Err(Error("wrong number of rule inputs".into()));
        }
        let mut stack = [0; MAX_STACK];
        let mut top = 0;
        for &op in &self.ops {
            match op {
                Op::Input(index) => {
                    stack[top] = inputs[usize::from(index)];
                    top += 1;
                }
                Op::Const(value) => {
                    stack[top] = value;
                    top += 1;
                }
                Op::Binary(operation) => {
                    top -= 1;
                    stack[top - 1] =
                        operation
                            .evaluate(stack[top - 1], stack[top])
                            .ok_or_else(|| {
                                Error("rule arithmetic overflow or division by zero".into())
                            })?;
                }
                Op::Select => {
                    top -= 2;
                    stack[top - 1] = if stack[top - 1] != 0 {
                        stack[top]
                    } else {
                        stack[top + 1]
                    };
                }
                Op::Guard => {
                    top -= 1;
                    if stack[top] == 0 {
                        return Ok(0);
                    }
                }
            }
        }
        Ok(stack[0])
    }

    /// Run `[[[input,...],expected],...]` assertions without host side effects.
    pub fn replay(&self, source: &str) -> Result<u32, Error> {
        measured(
            Feature::Replay,
            source.len(),
            || self.check_replay(source),
            |count| Work {
                assertions: u64::from(*count),
                ..Work::default()
            },
        )
    }

    fn check_replay(&self, source: &str) -> Result<u32, Error> {
        bound(source.len())?;
        let rows: Vec<(Vec<i64>, i64)> =
            serde_json::from_str(source).map_err(|error| Error(error.to_string()))?;
        if rows.is_empty() || rows.len() > crate::MAX_KEYS {
            return Err(Error("rule assertion count outside limits".into()));
        }
        for (index, (inputs, expected)) in rows.iter().enumerate() {
            let actual = self.run(inputs)?;
            if actual != *expected {
                return Err(Error(format!(
                    "rule assertion {} expected {expected}, got {actual}",
                    index + 1
                )));
            }
        }
        Ok(rows.len() as u32)
    }
}

fn read<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], Error> {
    let (prefix, rest) = bytes
        .split_first_chunk()
        .ok_or_else(|| Error("truncated rule".into()))?;
    *bytes = rest;
    Ok(*prefix)
}
