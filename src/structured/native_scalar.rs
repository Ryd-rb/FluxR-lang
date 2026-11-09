//! Lower a checked straight-line signed leaf through the existing rule JIT.
use super::compile::{Code, Op, Operand};
use super::*;
use crate::rules::{Binary as RuleBinary, MAX_INPUTS, Op as RuleOp, RuleProgram, native_jit};

#[derive(Debug)]
pub(super) struct Scalar {
    entry: native_jit::NativeRule,
    fuel: u64,
    cells: usize,
}

impl Code {
    pub(super) fn native_scalar(&self, inputs: &[Binding]) -> Option<&Scalar> {
        self.native
            .get_or_init(|| Scalar::compile(self, inputs))
            .as_ref()
    }
}

impl Scalar {
    fn compile(code: &Code, inputs: &[Binding]) -> Option<Self> {
        let count = u8::try_from(inputs.len() + code.params.len()).ok()?;
        if count > MAX_INPUTS
            || code.result != Type::I64
            || code.slots != code.params.len()
            || !inputs
                .iter()
                .map(|binding| &binding.ty)
                .chain(&code.params)
                .all(|ty| *ty == Type::I64)
        {
            return None;
        }
        let (end, body) = code.ops.split_last()?;
        if !matches!(end.op, Op::Return) || body.len() > 256 {
            return None;
        }
        let operand = |value: &Operand| match value {
            Operand::Const(Value::I64(value)) => Some(RuleOp::Const(*value)),
            Operand::Input(index) => Some(RuleOp::Input(u8::try_from(*index).ok()?)),
            Operand::Local(index) => Some(RuleOp::Input(u8::try_from(inputs.len() + index).ok()?)),
            _ => None,
        };
        let binary = |kind: Binary| {
            Some(RuleOp::Binary(match kind {
                Binary::Add => RuleBinary::Add,
                Binary::Sub => RuleBinary::Sub,
                Binary::Mul => RuleBinary::Mul,
                Binary::Div => RuleBinary::Div,
                Binary::Rem => RuleBinary::Rem,
                _ => return None,
            }))
        };
        let mut ops = Vec::with_capacity(body.len());
        let mut fuel = end.fuel;
        for instruction in body {
            fuel += instruction.fuel;
            match &instruction.op {
                Op::Operand(value) => ops.push(operand(value)?),
                Op::Binary(kind) => ops.push(binary(*kind)?),
                Op::BinaryOperands(kind, left, right) => {
                    ops.extend([operand(left)?, operand(right)?, binary(*kind)?]);
                }
                _ => return None,
            }
        }
        let rule = RuleProgram::compile(1, count, ops).ok()?;
        Some(Self {
            entry: native_jit::compile(&rule).ok()?,
            fuel,
            // Host inputs and args, one frame plus its slots, then one result.
            cells: usize::from(count) + code.slots + 2,
        })
    }

    pub(super) fn run(&self, call: &Invocation<'_>) -> Option<Result<Value, Error>> {
        // Use the interpreter for exact validation and error order when any
        // precondition fails, including a budget below the whole body.
        if call.limits.instructions < self.fuel
            || call.limits.cells < self.cells
            || call.limits.frames == 0
        {
            return None;
        }
        let count = call.inputs.len() + call.arguments.len();
        let mut raw = [0; MAX_INPUTS as usize];
        for (out, value) in raw.iter_mut().zip(call.inputs.iter().chain(call.arguments)) {
            let Value::I64(value) = value else {
                return None;
            };
            *out = *value;
        }
        Some(
            self.entry
                .evaluate(&raw[..count])
                .map(Value::I64)
                .map_err(|_| error("invalid or overflowing integer arithmetic")),
        )
    }
}
