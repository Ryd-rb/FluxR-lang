use super::compile::{Code, Op, Operand};
use super::*;

impl Module {
    pub fn run(&self, invocation: Invocation<'_>) -> Result<Value, Error> {
        self.run_with(invocation, &mut Workspace::default())
    }

    /// Reuse allocation capacity across calls. Values are released even when
    /// an invocation traps or exhausts its budget.
    pub fn run_with(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        let result = self.evaluate(invocation, workspace);
        workspace.operands.clear();
        workspace.locals.clear();
        workspace.frames.clear();
        result
    }

    fn evaluate(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        let function = *self
            .exports
            .get(invocation.export)
            .ok_or_else(|| error(format!("unknown export {}", invocation.export)))?;
        let code = &self.code[function];
        if invocation.inputs.len() != self.interface.inputs.len()
            || invocation.arguments.len() != code.params.len()
        {
            return Err(error("host argument or input count differs from interface"));
        }
        let mut machine = Machine {
            module: self,
            inputs: invocation.inputs,
            remaining: invocation.limits,
            operands: &mut workspace.operands,
            locals: &mut workspace.locals,
            frames: &mut workspace.frames,
        };
        for (value, binding) in invocation.inputs.iter().zip(&self.interface.inputs) {
            machine.check_value(value, &binding.ty, 0)?;
        }
        for (value, ty) in invocation.arguments.iter().zip(&code.params) {
            machine.check_value(value, ty, 0)?;
        }
        machine.operands.extend_from_slice(invocation.arguments);
        machine.call(function)?;
        let result = machine.execute()?;
        machine.check_value(&result, &code.result, 0)?;
        Ok(result)
    }
}

#[derive(Default)]
pub struct Workspace {
    operands: Vec<Value>,
    locals: Vec<Option<Value>>,
    frames: Vec<Frame>,
}

struct Frame {
    function: usize,
    pc: usize,
    slots: usize,
    stack: usize,
}

struct Machine<'a> {
    module: &'a Module,
    inputs: &'a [Value],
    remaining: Limits,
    operands: &'a mut Vec<Value>,
    locals: &'a mut Vec<Option<Value>>,
    frames: &'a mut Vec<Frame>,
}

impl Machine<'_> {
    fn charge_cells(&mut self, amount: usize) -> Result<(), Error> {
        self.remaining.cells = self
            .remaining
            .cells
            .checked_sub(amount)
            .ok_or_else(|| error("module cell budget exhausted"))?;
        Ok(())
    }

    fn check_value(&mut self, value: &Value, ty: &Type, depth: usize) -> Result<(), Error> {
        if depth >= MAX_DEPTH {
            return Err(error("host value nesting limit exceeded"));
        }
        self.charge_cells(1)?;
        let module = self.module;
        match (value, ty) {
            (Value::Unit, Type::Unit)
            | (Value::Bool(_), Type::Bool)
            | (Value::I64(_), Type::I64)
            | (Value::U64(_), Type::U64)
            | (Value::F32(_), Type::F32) => {}
            (Value::Record(fields), Type::Record(name))
                if fields.len() == module.records[name].fields.len() =>
            {
                for (value, binding) in fields.iter().zip(&module.records[name].fields) {
                    self.check_value(value, &binding.ty, depth + 1)?;
                }
            }
            (Value::Variant(tag, payload), Type::Enum(name)) => {
                let variant = module.enums[name]
                    .variants
                    .get(*tag)
                    .ok_or_else(|| error("unknown host enum tag"))?;
                match (payload, &variant.payload) {
                    (Some(value), Some(ty)) => self.check_value(value, ty, depth + 1)?,
                    (None, None) => {}
                    _ => return Err(error("host enum payload differs from interface")),
                }
            }
            (Value::List(values), Type::List(element)) => {
                for value in values.iter() {
                    self.check_value(value, element, depth + 1)?;
                }
            }
            _ => return Err(error(format!("host value differs from {ty:?}"))),
        }
        Ok(())
    }

    fn call(&mut self, function: usize) -> Result<(), Error> {
        let Code { params, slots, .. } = self.module.code[function].as_ref();
        if self.frames.len() >= self.remaining.frames {
            return Err(error("module frame limit exceeded"));
        }
        self.charge_cells(slots + 1)?;
        let frame = Frame {
            function,
            pc: 0,
            slots: self.locals.len(),
            stack: self.operands.len() - params.len(),
        };
        self.locals
            .extend(self.operands.drain(frame.stack..).map(Some));
        self.locals.resize_with(frame.slots + slots, || None);
        self.frames.push(frame);
        Ok(())
    }

    fn pop(&mut self) -> Value {
        self.operands.pop().expect("checked operand stack")
    }

    fn local(&self, index: usize) -> Result<&Value, Error> {
        self.locals[index]
            .as_ref()
            .ok_or_else(|| error("read of uninitialized local"))
    }

    fn local_mut(&mut self, index: usize) -> Result<&mut Value, Error> {
        self.locals[index]
            .as_mut()
            .ok_or_else(|| error("update of uninitialized local"))
    }

    fn operand<'a>(&'a self, source: &'a Operand, slots: usize) -> Result<&'a Value, Error> {
        Ok(match source {
            Operand::Const(value) => value,
            Operand::Input(index) => &self.inputs[*index],
            Operand::Local(index) => self.local(slots + index)?,
            Operand::LocalField(index, field) => {
                let Value::Record(fields) = self.local(slots + index)? else {
                    unreachable!("typed record");
                };
                &fields[*field]
            }
        })
    }

    fn record_update(&mut self, index: usize, field: usize) -> Result<(), Error> {
        let value = self.pop();
        let Value::Record(fields) = self.local(index)? else {
            unreachable!("typed record");
        };
        if Arc::strong_count(fields) != 1 {
            self.charge_cells(fields.len())?;
        }
        let Value::Record(fields) = self.local_mut(index)? else {
            unreachable!("typed record");
        };
        Arc::make_mut(fields)[field] = value;
        Ok(())
    }

    fn list_update(&mut self, index: usize, value: Option<Value>) -> Result<(), Error> {
        let Value::List(values) = self.local(index)? else {
            unreachable!("typed list");
        };
        let copy = if Arc::strong_count(values) == 1 {
            0
        } else {
            values.len()
        };
        self.charge_cells(copy + usize::from(value.is_some()))?;
        let Value::List(values) = self.local_mut(index)? else {
            unreachable!("typed list");
        };
        let values = Arc::make_mut(values);
        if let Some(value) = value {
            values.push(value);
        } else if values.pop().is_none() {
            return Err(error("pop from empty list"));
        }
        Ok(())
    }

    fn execute(&mut self) -> Result<Value, Error> {
        let module = self.module;
        loop {
            let frame = self.frames.last_mut().expect("active frame");
            let instruction = &module.code[frame.function].ops[frame.pc];
            self.remaining.instructions = self
                .remaining
                .instructions
                .checked_sub(instruction.fuel)
                .ok_or_else(|| error("module instruction budget exhausted"))?;
            let op = &instruction.op;
            let slots = frame.slots;
            frame.pc += 1;
            let value = match op {
                Op::Operand(source) => self.operand(source, slots)?.clone(),
                Op::Store(index) => {
                    self.locals[slots + index] = Some(self.pop());
                    continue;
                }
                Op::StoreField(index, field) => {
                    self.record_update(slots + index, *field)?;
                    continue;
                }
                Op::Binary(kind) => {
                    let right = self.pop();
                    binary(*kind, &self.pop(), &right)?
                }
                Op::BinaryOperands(kind, left, right) => binary(
                    *kind,
                    self.operand(left, slots)?,
                    self.operand(right, slots)?,
                )?,
                Op::Unary(kind) => unary(*kind, self.pop()),
                Op::Record(fields) => {
                    self.charge_cells(fields + 1)?;
                    let fields = self.operands.split_off(self.operands.len() - fields);
                    Value::Record(fields.into())
                }
                Op::Field(index) => {
                    let Value::Record(fields) = self.pop() else {
                        unreachable!("typed record");
                    };
                    fields[*index].clone()
                }
                Op::Variant(tag, payload) => {
                    self.charge_cells(1)?;
                    Value::Variant(*tag, payload.then(|| Arc::new(self.pop())))
                }
                Op::IsVariant(want) => {
                    let Value::Variant(tag, _) = self.pop() else {
                        unreachable!("typed enum");
                    };
                    Value::Bool(tag == *want)
                }
                Op::Payload(want) => {
                    let Value::Variant(tag, Some(payload)) = self.pop() else {
                        return Err(error("enum payload does not match variant"));
                    };
                    if tag != *want {
                        return Err(error("enum payload does not match variant"));
                    }
                    Arc::unwrap_or_clone(payload)
                }
                Op::List(len) => {
                    self.charge_cells(len + 1)?;
                    let values = self.operands.split_off(self.operands.len() - len);
                    Value::List(Arc::new(values))
                }
                Op::Empty => {
                    let Value::List(values) = self.pop() else {
                        unreachable!("typed list");
                    };
                    Value::Bool(values.is_empty())
                }
                Op::Last => {
                    let Value::List(values) = self.pop() else {
                        unreachable!("typed list");
                    };
                    values
                        .last()
                        .cloned()
                        .ok_or_else(|| error("last from empty list"))?
                }
                Op::Index => {
                    let Value::U64(index) = self.pop() else {
                        unreachable!("typed index");
                    };
                    let Value::List(values) = self.pop() else {
                        unreachable!("typed list");
                    };
                    usize::try_from(index)
                        .ok()
                        .and_then(|index| values.get(index))
                        .cloned()
                        .ok_or_else(|| error("list index out of bounds"))?
                }
                Op::ArgMinF32 => {
                    let Value::List(values) = self.pop() else {
                        unreachable!("typed list");
                    };
                    self.remaining.instructions = self
                        .remaining
                        .instructions
                        .checked_sub(values.len() as u64)
                        .ok_or_else(|| error("module instruction budget exhausted"))?;
                    let mut best = None;
                    for (index, value) in values.iter().enumerate() {
                        let Value::F32(bits) = value else {
                            unreachable!("typed float");
                        };
                        let value = f32::from_bits(*bits);
                        if best.is_none_or(|(_, old)| value < old) {
                            best = Some((index, value));
                        }
                    }
                    Value::U64(best.map_or(values.len(), |(index, _)| index) as u64)
                }
                Op::Push(index) => {
                    let value = self.pop();
                    self.list_update(slots + index, Some(value))?;
                    continue;
                }
                Op::Pop(index) => {
                    self.list_update(slots + index, None)?;
                    continue;
                }
                Op::Call(function) => {
                    self.call(*function)?;
                    continue;
                }
                Op::Jump(target) => {
                    frame.pc = *target;
                    continue;
                }
                Op::JumpFalse(target) | Op::JumpTrue(target) => {
                    let take_if = matches!(op, Op::JumpTrue(_));
                    let Value::Bool(condition) = self.pop() else {
                        unreachable!("typed branch");
                    };
                    if condition == take_if {
                        self.frames.last_mut().expect("active frame").pc = *target;
                    }
                    continue;
                }
                Op::Return => {
                    let value = self.pop();
                    let frame = self.frames.pop().expect("active frame");
                    self.locals.truncate(frame.slots);
                    self.operands.truncate(frame.stack);
                    if self.frames.is_empty() {
                        return Ok(value);
                    }
                    value
                }
            };
            self.operands.push(value);
        }
    }
}

fn unary(kind: Unary, value: Value) -> Value {
    match (kind, value) {
        (Unary::Not, Value::Bool(value)) => Value::Bool(!value),
        (Unary::AbsF32, Value::F32(bits)) => Value::F32(bits & 0x7fff_ffff),
        (Unary::U64ToF32, Value::U64(value)) => Value::F32((value as f32).to_bits()),
        _ => unreachable!("checked unary operation"),
    }
}

fn binary(kind: Binary, left: &Value, right: &Value) -> Result<Value, Error> {
    use Binary::*;
    fn checked<T>(value: Option<T>, make: fn(T) -> Value) -> Result<Value, Error> {
        value
            .map(make)
            .ok_or_else(|| error("invalid or overflowing integer arithmetic"))
    }
    macro_rules! integer {
        ($left:ident, $right:ident, $variant:path) => {{
            let ($left, $right) = (*$left, *$right);
            match kind {
                Add => checked($left.checked_add($right), $variant),
                Sub => checked($left.checked_sub($right), $variant),
                Mul => checked($left.checked_mul($right), $variant),
                Div => checked($left.checked_div($right), $variant),
                Rem => checked($left.checked_rem($right), $variant),
                Eq => Ok(Value::Bool($left == $right)),
                Lt => Ok(Value::Bool($left < $right)),
                Le => Ok(Value::Bool($left <= $right)),
                _ => unreachable!("checked integer operation"),
            }
        }};
    }
    match (left, right) {
        (Value::I64(left), Value::I64(right)) => integer!(left, right, Value::I64),
        (Value::U64(left), Value::U64(right)) => integer!(left, right, Value::U64),
        (Value::Bool(left), Value::Bool(right)) => {
            debug_assert!(matches!(kind, Eq));
            Ok(Value::Bool(left == right))
        }
        (Value::F32(left), Value::F32(right)) => {
            let (left, right) = (f32::from_bits(*left), f32::from_bits(*right));
            let value = match kind {
                Add => left + right,
                Sub => left - right,
                Mul => left * right,
                Div => left / right,
                Eq => return Ok(Value::Bool(left == right)),
                Lt => return Ok(Value::Bool(left < right)),
                Le => return Ok(Value::Bool(left <= right)),
                _ => unreachable!("checked binary32 operation"),
            };
            Ok(Value::F32(if value.is_nan() {
                0x7fc0_0000
            } else {
                value.to_bits()
            }))
        }
        _ => unreachable!("checked binary operation"),
    }
}
