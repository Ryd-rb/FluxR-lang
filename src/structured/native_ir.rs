//! Typed, register-based control flow lowered from the checked VM instruction stream.
//! Whole invocations are speculative: a decline replays the original VM.
use super::compile::{Code, Op, Operand};
use super::*;

const MAX_EXPANDED_OPS: usize = 512;
const MAX_NATIVE_DEPTH: usize = 8;
pub(super) const MAX_LANES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    I64,
    Bool,
    // The first lane holds the original host index (zero after a write).
    Record(usize),
}

impl Shape {
    fn for_type(ty: &Type, module: &Module) -> Option<Self> {
        match ty {
            Type::I64 => Some(Self::I64),
            Type::Bool => Some(Self::Bool),
            Type::Record(name) => {
                let fields = &module.records.get(name)?.fields;
                (fields.len() <= 4 && fields.iter().all(|field| field.ty == Type::I64))
                    .then_some(Self::Record(fields.len()))
            }
            _ => None,
        }
    }

    pub(super) fn lanes(self) -> usize {
        match self {
            Self::Record(fields) => fields + 1,
            _ => 1,
        }
    }

    fn pack(self, value: &Value, origin: usize, out: &mut [i64]) -> Option<()> {
        match (self, value) {
            (Self::I64, Value::I64(value)) => out[0] = *value,
            (Self::Bool, Value::Bool(value)) => out[0] = i64::from(*value),
            (Self::Record(count), Value::Record(fields)) if count == fields.len() => {
                out[0] = origin as i64;
                for (value, lane) in fields.iter().zip(&mut out[1..]) {
                    let Value::I64(value) = value else {
                        return None;
                    };
                    *lane = *value;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

pub(super) type Temp = usize;

#[derive(Debug)]
pub(super) enum Step {
    Const(i64),
    Input(usize),
    Load(usize),
    Store(usize, Temp),
    Update(usize, usize, Temp),
    Field(Temp, usize),
    Binary(Binary, Temp, Temp),
    Call(usize, Vec<Temp>),
}

#[derive(Debug)]
pub(super) enum Exit {
    Return(Temp),
    Jump(usize),
    Branch(Temp, usize, usize),
}

#[derive(Debug)]
pub(super) struct Block {
    pub(super) fuel: u64,
    pub(super) cells: usize,
    pub(super) steps: Vec<Step>,
    pub(super) exit: Exit,
}

#[derive(Debug)]
pub(super) struct Function {
    pub(super) slots: Vec<Shape>,
    pub(super) params: usize,
    pub(super) result: Shape,
    pub(super) blocks: Vec<Block>,
}

#[derive(Debug)]
pub(super) struct Plan {
    pub(super) entry: usize,
    pub(super) functions: Vec<Option<Function>>,
    pub(super) boundary: Boundary,
}

#[derive(Debug)]
pub(super) struct Boundary {
    pub(super) inputs: Vec<Shape>,
    pub(super) args: Vec<Shape>,
    pub(super) result: Shape,
    boundary_cells: usize,
    frames: usize,
    signed_io: bool,
    single_signed_record: bool,
    // One-block call graphs can have all their metering performed at admission.
    pub(super) fixed: Option<(u64, usize)>,
}

#[derive(Clone, Copy)]
struct Work {
    expanded: usize,
    frames: usize,
    fixed: Option<(u64, usize)>,
}

impl Plan {
    pub(super) fn lower(module: &Module, entry: usize) -> Option<Self> {
        let shape = |ty: &Type| Shape::for_type(ty, module);
        let inputs = module
            .interface
            .inputs
            .iter()
            .map(|b| shape(&b.ty))
            .collect::<Option<Vec<_>>>()?;
        let code = module.code.get(entry)?;
        let args = code.params.iter().map(shape).collect::<Option<Vec<_>>>()?;
        let result = shape(&code.result)?;
        let input_lanes = inputs
            .iter()
            .chain(&args)
            .map(|shape| shape.lanes())
            .sum::<usize>();
        if input_lanes > MAX_LANES {
            return None;
        }
        let mut functions: Vec<_> = (0..module.code.len()).map(|_| None).collect();
        let mut work = vec![None; functions.len()];
        let cost = lower_function(module, entry, &mut functions, &mut work, 1)?;
        let signed_io =
            result == Shape::I64 && inputs.iter().chain(&args).all(|&shape| shape == Shape::I64);
        let single_signed_record = inputs.is_empty() && args == [Shape::Record(1), Shape::I64];
        Some(Self {
            entry,
            functions,
            boundary: Boundary {
                inputs,
                args,
                result,
                boundary_cells: input_lanes + result.lanes(),
                frames: cost.frames,
                signed_io,
                single_signed_record,
                fixed: cost.fixed,
            },
        })
    }

    #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
    pub(super) fn function(&self, index: usize) -> &Function {
        self.functions[index].as_ref().expect("lowered call target")
    }
}

impl Boundary {
    #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
    pub(super) fn fits_lanes(&self, lanes: usize) -> bool {
        let output_lanes = self.result.lanes();
        output_lanes <= lanes && self.boundary_cells - output_lanes <= lanes
    }

    pub(super) fn run<const LANES: usize>(
        &self,
        call: &Invocation<'_>,
        kernel: impl FnOnce(&[i64; LANES], &mut [i64; LANES], u64, u64) -> u32,
    ) -> Option<Value> {
        if call.limits.frames < self.frames
            || call.inputs.len() != self.inputs.len()
            || call.arguments.len() != self.args.len()
        {
            return None;
        }
        let cells = call.limits.cells.checked_sub(self.boundary_cells)?;
        let fuel = call.limits.instructions;
        if let Some((need_fuel, need_cells)) = self.fixed
            && (fuel < need_fuel || cells < need_cells)
        {
            return None;
        }
        let mut raw = [0; LANES];
        let mut out = [0; LANES];
        if self.signed_io {
            let (inputs, arguments) = raw.split_at_mut(call.inputs.len());
            pack_signed(inputs, call.inputs)?;
            pack_signed(arguments, call.arguments)?;
            return (kernel(&raw, &mut out, fuel, cells as u64) == 0).then(|| Value::I64(out[0]));
        }
        if self.single_signed_record {
            let [Value::Record(fields), Value::I64(arg)] = call.arguments else {
                return None;
            };
            let [Value::I64(field)] = fields.as_ref() else {
                return None;
            };
            raw[..3].copy_from_slice(&[1, *field, *arg]);
        } else {
            let mut offset = 0;
            pack_values(&self.inputs, call.inputs, 1, &mut raw, &mut offset)?;
            pack_values(
                &self.args,
                call.arguments,
                call.inputs.len() + 1,
                &mut raw,
                &mut offset,
            )?;
        }
        if kernel(&raw, &mut out, fuel, cells as u64) != 0 {
            return None;
        }
        Some(match self.result {
            Shape::I64 => Value::I64(out[0]),
            Shape::Bool => Value::Bool(out[0] != 0),
            Shape::Record(fields) if out[0] == 0 => {
                Value::Record(out[1..=fields].iter().copied().map(Value::I64).collect())
            }
            Shape::Record(_) => {
                let index = usize::try_from(out[0]).ok()?.checked_sub(1)?;
                let original = if index < call.inputs.len() {
                    &call.inputs[index]
                } else {
                    call.arguments.get(index - call.inputs.len())?
                };
                let Value::Record(fields) = original else {
                    return None;
                };
                Value::Record(Arc::clone(fields))
            }
        })
    }
}

#[inline]
fn pack_values(
    shapes: &[Shape],
    values: &[Value],
    origin: usize,
    raw: &mut [i64],
    offset: &mut usize,
) -> Option<()> {
    for (i, (&shape, value)) in shapes.iter().zip(values).enumerate() {
        shape.pack(value, origin + i, &mut raw[*offset..])?;
        *offset += shape.lanes();
    }
    Some(())
}

#[inline]
fn pack_signed(lanes: &mut [i64], values: &[Value]) -> Option<()> {
    for (lane, value) in lanes.iter_mut().zip(values) {
        let Value::I64(value) = value else {
            return None;
        };
        *lane = *value;
    }
    Some(())
}

#[cfg(test)]
#[test]
fn single_signed_record_admission() {
    let module =
        Module::compile_compact("v2 (rec R x:i64) (pub keep (s:R d:i64) R () (ret s))").unwrap();
    let plan = Plan::lower(&module, module.exports["keep"]).unwrap();
    let state: Arc<[Value]> = [Value::I64(-23)].into();
    let args = [Value::Record(state.clone()), Value::I64(i64::MIN)];
    let call = Invocation {
        export: "keep",
        inputs: &[],
        arguments: &args,
        limits: Limits {
            instructions: 2,
            cells: 8,
            frames: 1,
        },
    };
    let result = plan
        .boundary
        .run::<MAX_LANES>(&call, |raw, out, fuel, cells| {
            assert_eq!(&raw[..3], &[1, -23, i64::MIN]);
            assert!(raw[3..].iter().all(|&lane| lane == 0));
            assert_eq!((fuel, cells), (2, 3));
            out[..2].copy_from_slice(&raw[..2]);
            0
        });
    let Some(Value::Record(result)) = result else {
        panic!("valid record must reach the kernel");
    };
    assert!(Arc::ptr_eq(&result, &state));

    let reject = |call: Invocation<'_>| {
        assert!(
            plan.boundary
                .run::<MAX_LANES>(&call, |_, _, _, _| panic!(
                    "invalid call reached the kernel"
                ))
                .is_none()
        );
    };
    for first in [
        Value::I64(0),
        Value::Record([].into()),
        Value::Record([Value::Bool(false)].into()),
        Value::Record([Value::I64(0), Value::I64(1)].into()),
    ] {
        reject(Invocation {
            arguments: &[first, Value::I64(0)],
            ..call
        });
    }
    for arguments in [
        &args[..1],
        &[args[0].clone(), Value::Bool(true)],
        &[args[0].clone(), args[1].clone(), Value::I64(0)],
    ] {
        reject(Invocation { arguments, ..call });
    }
    reject(Invocation {
        inputs: &[Value::I64(0)],
        ..call
    });
    for limits in [
        Limits {
            instructions: 1,
            ..call.limits
        },
        Limits {
            cells: 7,
            ..call.limits
        },
        Limits {
            frames: 0,
            ..call.limits
        },
    ] {
        reject(Invocation { limits, ..call });
    }
}

fn lower_function(
    module: &Module,
    index: usize,
    functions: &mut [Option<Function>],
    work: &mut [Option<Work>],
    depth: usize,
) -> Option<Work> {
    if depth > MAX_NATIVE_DEPTH {
        return None;
    }
    if let Some(cost) = work[index] {
        return (depth + cost.frames - 1 <= MAX_NATIVE_DEPTH).then_some(cost);
    }
    let code = &module.code[index];
    if code.ops.len() > MAX_EXPANDED_OPS {
        return None;
    }
    let slots = code
        .slot_types
        .iter()
        .map(|ty| Shape::for_type(ty, module))
        .collect::<Option<Vec<_>>>()?;
    if slots.iter().map(|shape| shape.lanes()).sum::<usize>() > MAX_LANES {
        return None;
    }
    let result = Shape::for_type(&code.result, module)?;
    let blocks = blocks(code, module, &slots, result)?;
    let mut cost = Work {
        expanded: code.ops.len(),
        frames: 1,
        fixed: (blocks.len() == 1).then(|| (blocks[0].fuel, blocks[0].cells)),
    };
    for block in &blocks {
        for step in &block.steps {
            if let Step::Call(callee, _) = step {
                let nested = lower_function(module, *callee, functions, work, depth + 1)?;
                cost.expanded += nested.expanded;
                if cost.expanded > MAX_EXPANDED_OPS {
                    return None;
                }
                cost.frames = cost.frames.max(nested.frames + 1);
                cost.fixed = cost
                    .fixed
                    .zip(nested.fixed)
                    .map(|((fuel, cells), (nf, nc))| (fuel + nf, cells + nc));
            }
        }
    }
    work[index] = Some(cost);
    functions[index] = Some(Function {
        slots,
        params: code.params.len(),
        result,
        blocks,
    });
    Some(cost)
}

fn blocks(code: &Code, module: &Module, slots: &[Shape], result: Shape) -> Option<Vec<Block>> {
    let end = code.ops.len();
    let mut starts = vec![0, end];
    for (pc, inst) in code.ops.iter().enumerate() {
        match inst.op {
            Op::Jump(to) | Op::JumpTrue(to) | Op::JumpFalse(to) => {
                if to >= end {
                    return None;
                }
                starts.extend([to, pc + 1]);
            }
            Op::Return => starts.push(pc + 1),
            _ => {}
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let block_at = |pc| {
        starts
            .binary_search(&pc)
            .ok()
            .filter(|&i| i + 1 < starts.len())
    };
    starts
        .windows(2)
        .enumerate()
        .map(|(index, pcs)| {
            let mut builder = BlockBuilder {
                steps: Vec::new(),
                types: Vec::new(),
                stack: Vec::new(),
                module,
                slots,
            };
            let mut fuel = 0;
            // Entering the first block from a backedge does not enter a new frame.
            let mut cells = 0;
            let mut exit = None;
            for inst in &code.ops[pcs[0]..pcs[1]] {
                fuel += inst.fuel;
                let shape = match &inst.op {
                    Op::Operand(value) => {
                        let temp = builder.operand(value)?;
                        builder.stack.push(temp);
                        continue;
                    }
                    Op::Store(slot) => {
                        let value = builder.pop(slots[*slot])?;
                        builder.emit(Step::Store(*slot, value), slots[*slot]);
                        continue;
                    }
                    Op::StoreField(slot, field) => {
                        let Shape::Record(fields) = slots[*slot] else {
                            return None;
                        };
                        let value = builder.pop(Shape::I64)?;
                        cells += fields;
                        builder.emit(Step::Update(*slot, *field, value), slots[*slot]);
                        continue;
                    }
                    Op::Field(field) => {
                        let record = builder.stack.pop()?;
                        let Shape::Record(fields) = builder.types[record] else {
                            return None;
                        };
                        if *field >= fields {
                            return None;
                        }
                        (Step::Field(record, *field), Shape::I64)
                    }
                    Op::Binary(kind) => {
                        let right = builder.stack.pop()?;
                        let left = builder.stack.pop()?;
                        builder.binary(*kind, left, right)?
                    }
                    Op::BinaryOperands(kind, left, right) => {
                        let left = builder.operand(left)?;
                        let right = builder.operand(right)?;
                        builder.binary(*kind, left, right)?
                    }
                    Op::Call(callee) => {
                        let target = &module.code[*callee];
                        let mut args = Vec::with_capacity(target.params.len());
                        for ty in target.params.iter().rev() {
                            args.push(builder.pop(Shape::for_type(ty, module)?)?);
                        }
                        args.reverse();
                        (
                            Step::Call(*callee, args),
                            Shape::for_type(&target.result, module)?,
                        )
                    }
                    Op::Return => {
                        exit = Some(Exit::Return(builder.pop(result)?));
                        break;
                    }
                    Op::Jump(to) => {
                        exit = Some(Exit::Jump(block_at(*to)?));
                        break;
                    }
                    Op::JumpFalse(to) | Op::JumpTrue(to) => {
                        let condition = builder.pop(Shape::Bool)?;
                        let target = block_at(*to)?;
                        let next = block_at(pcs[1])?;
                        let (yes, no) = if matches!(inst.op, Op::JumpTrue(_)) {
                            (target, next)
                        } else {
                            (next, target)
                        };
                        exit = Some(Exit::Branch(condition, yes, no));
                        break;
                    }
                    _ => return None,
                };
                let temp = builder.emit(shape.0, shape.1);
                builder.stack.push(temp);
            }
            if !builder.stack.is_empty() {
                // Short-circuit expressions carry operands across edges. Leave
                // those in the VM until the IR supports block arguments for them.
                return None;
            }
            let exit = match exit {
                Some(exit) => exit,
                None => Exit::Jump(block_at(starts[index + 1])?),
            };
            Some(Block {
                fuel,
                cells,
                steps: builder.steps,
                exit,
            })
        })
        .collect::<Option<Vec<_>>>()
        .map(|mut blocks| {
            // Use a dedicated entry block so loops targeting instruction zero
            // cannot repeatedly charge frame setup.
            if blocks.len() == 1 && matches!(blocks[0].exit, Exit::Return(_)) {
                blocks[0].cells += code.slots + 1;
            } else {
                blocks.iter_mut().for_each(|block| match &mut block.exit {
                    Exit::Jump(target) => *target += 1,
                    Exit::Branch(_, yes, no) => {
                        *yes += 1;
                        *no += 1;
                    }
                    Exit::Return(_) => {}
                });
                blocks.insert(
                    0,
                    Block {
                        fuel: 0,
                        cells: code.slots + 1,
                        steps: Vec::new(),
                        exit: Exit::Jump(1),
                    },
                );
            }
            blocks
        })
}

struct BlockBuilder<'a> {
    steps: Vec<Step>,
    types: Vec<Shape>,
    stack: Vec<Temp>,
    module: &'a Module,
    slots: &'a [Shape],
}

impl BlockBuilder<'_> {
    fn emit(&mut self, step: Step, shape: Shape) -> Temp {
        let temp = self.steps.len();
        self.steps.push(step);
        self.types.push(shape);
        temp
    }

    fn pop(&mut self, shape: Shape) -> Option<Temp> {
        let temp = self.stack.pop()?;
        (self.types[temp] == shape).then_some(temp)
    }

    fn operand(&mut self, operand: &Operand) -> Option<Temp> {
        let (step, shape) = match operand {
            Operand::Const(Value::I64(value)) => (Step::Const(*value), Shape::I64),
            Operand::Const(Value::Bool(value)) => (Step::Const(i64::from(*value)), Shape::Bool),
            Operand::Input(index) => (
                Step::Input(*index),
                Shape::for_type(&self.module.interface.inputs[*index].ty, self.module)?,
            ),
            Operand::Local(slot) => (Step::Load(*slot), self.slots[*slot]),
            Operand::LocalField(slot, field) => {
                let record = self.emit(Step::Load(*slot), self.slots[*slot]);
                (Step::Field(record, *field), Shape::I64)
            }
            _ => return None,
        };
        Some(self.emit(step, shape))
    }

    fn binary(&self, kind: Binary, left: Temp, right: Temp) -> Option<(Step, Shape)> {
        use Binary::*;
        let left_type = self.types[left];
        if left_type != self.types[right] {
            return None;
        }
        let result = match (kind, left_type) {
            (Add | Sub | Mul | Div | Rem, Shape::I64) => Shape::I64,
            (Eq, Shape::I64 | Shape::Bool) | (Lt | Le, Shape::I64) => Shape::Bool,
            _ => return None,
        };
        Some((Step::Binary(kind, left, right), result))
    }
}
