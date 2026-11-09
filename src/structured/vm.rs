use super::compile::{Code, Instruction, Op, Operand};
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
        self.run_in(invocation, workspace, true)
    }

    /// Execute with the portable oracle, without preparing or entering native code.
    pub fn run_portable_with(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        self.run_in(invocation, workspace, false)
    }

    fn run_in(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
        _native: bool,
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
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        if _native
            && let Some(result) = code
                .native
                .get()
                .and_then(Option::as_ref)
                .and_then(|native| native.run(&invocation))
        {
            return result;
        }
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        if _native
            && let Some(result) = self.native[function]
                .get()
                .and_then(Option::as_ref)
                .and_then(|native| native.run(&invocation))
        {
            return Ok(result);
        }
        self.run_portable_function(invocation, workspace, function, code, _native)
    }

    /// Prepare the configured native tier for a validated export. An
    /// unsupported export continues to run in the portable interpreter.
    pub fn prepare_native(&self, export: &str) -> bool {
        self.exports
            .get(export)
            .is_some_and(|&index| self.prepare_function(index))
    }

    fn prepare_function(&self, _function: usize) -> bool {
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        if self.code[_function]
            .native_scalar(&self.interface.inputs)
            .is_some()
        {
            return true;
        }
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        return self.native[_function]
            .get_or_init(|| super::typed_jit::Native::compile(self, _function))
            .is_some();
        #[cfg(not(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64")))]
        false
    }

    // Only the portable evaluator uses scratch space. Keep its frame and
    // cleanup outside the successful native path.
    fn run_portable_function(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
        function: usize,
        code: &Code,
        _native: bool,
    ) -> Result<Value, Error> {
        let result = self.evaluate_portable(invocation, workspace, function, code);
        workspace.operands.clear();
        workspace.locals.clear();
        workspace.frames.clear();
        // A rejected invocation cannot trigger automatic native compilation.
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        if _native && result.is_ok() {
            self.prepare_function(function);
        }
        result
    }

    fn evaluate_portable(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
        function: usize,
        code: &Code,
    ) -> Result<Value, Error> {
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
        if machine.remaining.frames == 0 {
            return Err(error("module frame limit exceeded"));
        }
        machine.charge_cells(code.slots + 1)?;
        let result =
            if let Some((values, scan_fuel, return_fuel)) = scan_leaf(&code.ops, &invocation) {
                machine.charge_fuel(scan_fuel)?;
                let index = machine.scan_min_f32(values)?;
                machine.charge_fuel(return_fuel)?;
                Value::U64(index)
            } else {
                machine
                    .locals
                    .extend(invocation.arguments.iter().cloned().map(Some));
                machine.locals.resize_with(code.slots, || None);
                machine.frames.push(Frame {
                    function,
                    pc: 0,
                    slots: 0,
                    stack: 0,
                });
                machine.execute()?
            };
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

fn check_host_depth(depth: usize) -> Result<(), Error> {
    if depth >= MAX_DEPTH {
        return Err(error("host value nesting limit exceeded"));
    }
    Ok(())
}

pub(super) fn scan_effect(ops: &[Instruction]) -> Option<(&Operand, u64, u64)> {
    let [load, scan, ret] = ops else {
        return None;
    };
    if !matches!(scan.op, Op::ArgMinF32) || !matches!(ret.op, Op::Return) {
        return None;
    }
    let Op::Operand(operand) = &load.op else {
        return None;
    };
    Some((operand, load.fuel + scan.fuel, ret.fuel))
}

fn scan_leaf<'a>(ops: &[Instruction], call: &Invocation<'a>) -> Option<(&'a [Value], u64, u64)> {
    let (load, scan_fuel, return_fuel) = scan_effect(ops)?;
    // After boundary admission, these loads cannot trap. Other locals still
    // go through the interpreter to preserve uninitialized-read precedence.
    let value = match load {
        Operand::Input(index) => &call.inputs[*index],
        Operand::Local(index) => call.arguments.get(*index)?,
        _ => return None,
    };
    let Value::List(values) = value else {
        unreachable!("typed list");
    };
    Some((values, scan_fuel, return_fuel))
}

pub(super) fn min_f32_index(values: &[Value]) -> u64 {
    let mut best = None;
    for (index, value) in values.iter().enumerate() {
        let Value::F32(bits) = value else {
            unreachable!("typed float");
        };
        let value = f32::from_bits(*bits);
        // Strict less-than retains the first tie and leaves NaNs unordered.
        if best.is_none_or(|(_, old)| value < old) {
            best = Some((index, value));
        }
    }
    best.map_or(values.len(), |(index, _)| index) as u64
}

impl Machine<'_> {
    fn charge_fuel(&mut self, amount: u64) -> Result<(), Error> {
        self.remaining.instructions = self
            .remaining
            .instructions
            .checked_sub(amount)
            .ok_or_else(|| error("module instruction budget exhausted"))?;
        Ok(())
    }

    fn charge_cells(&mut self, amount: usize) -> Result<(), Error> {
        self.remaining.cells = self
            .remaining
            .cells
            .checked_sub(amount)
            .ok_or_else(|| error("module cell budget exhausted"))?;
        Ok(())
    }

    fn check_value(&mut self, value: &Value, ty: &Type, depth: usize) -> Result<(), Error> {
        check_host_depth(depth)?;
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
            (Value::List(values), Type::List(element)) if **element == Type::F32 => {
                self.admit_scalar_elements(values, depth + 1, "F32", |value| {
                    matches!(value, Value::F32(_))
                })?;
            }
            (Value::List(values), Type::List(element)) if **element == Type::I64 => {
                self.admit_scalar_elements(values, depth + 1, "I64", |value| {
                    matches!(value, Value::I64(_))
                })?;
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

    // The caller already checked the outer list's depth, cell and tag.
    fn admit_scalar_elements(
        &mut self,
        values: &[Value],
        depth: usize,
        ty: &'static str,
        matches_type: impl FnMut(&Value) -> bool,
    ) -> Result<(), Error> {
        if values.is_empty() {
            return Ok(());
        }
        check_host_depth(depth)?;
        // A tag error can win only at an element whose cell can be charged.
        let affordable = values.len().min(self.remaining.cells);
        if !values[..affordable].iter().all(matches_type) {
            return Err(error(format!("host value differs from {ty}")));
        }
        self.charge_cells(values.len())
    }

    fn scan_min_f32(&mut self, values: &[Value]) -> Result<u64, Error> {
        self.charge_fuel(values.len() as u64)?;
        Ok(min_f32_index(values))
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

    fn record_update(&mut self, index: usize, field: usize, value: Value) -> Result<(), Error> {
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
        let activate =
            |frame: &Frame| (&module.code[frame.function].ops[..], frame.slots, frame.pc);
        let (mut ops, mut slots, mut pc) = activate(self.frames.last().expect("active frame"));
        loop {
            let instruction = &ops[pc];
            self.charge_fuel(instruction.fuel)?;
            let op = &instruction.op;
            pc += 1;
            let value = match op {
                Op::Operand(source) => self.operand(source, slots)?.clone(),
                Op::Store(index) => {
                    self.locals[slots + index] = Some(self.pop());
                    continue;
                }
                Op::StoreField(index, field) => {
                    let value = self.pop();
                    self.record_update(slots + index, *field, value)?;
                    continue;
                }
                Op::Binary(kind) => {
                    let right = self.pop();
                    binary(*kind, &self.pop(), &right)?
                }
                Op::BinaryOperands(kind, left, right) => {
                    let value = binary(
                        *kind,
                        self.operand(left, slots)?,
                        self.operand(right, slots)?,
                    )?;
                    // Preserve the second instruction's charge after arithmetic
                    // so an arithmetic failure still takes precedence.
                    if let Some(next) = ops.get(pc) {
                        match &next.op {
                            Op::Store(index) => {
                                self.charge_fuel(next.fuel)?;
                                self.locals[slots + index] = Some(value);
                                pc += 1;
                                continue;
                            }
                            Op::StoreField(index, field) => {
                                self.charge_fuel(next.fuel)?;
                                self.record_update(slots + index, *field, value)?;
                                pc += 1;
                                continue;
                            }
                            Op::JumpFalse(target) | Op::JumpTrue(target) => {
                                self.charge_fuel(next.fuel)?;
                                let Value::Bool(condition) = value else {
                                    unreachable!("typed branch");
                                };
                                pc = if condition == matches!(next.op, Op::JumpTrue(_)) {
                                    *target
                                } else {
                                    pc + 1
                                };
                                continue;
                            }
                            _ => {}
                        }
                    }
                    value
                }
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
                    Value::U64(self.scan_min_f32(&values)?)
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
                    self.frames.last_mut().expect("active frame").pc = pc;
                    self.call(*function)?;
                    (ops, slots, pc) = activate(self.frames.last().expect("callee frame"));
                    continue;
                }
                Op::Jump(target) => {
                    pc = *target;
                    continue;
                }
                Op::JumpFalse(target) | Op::JumpTrue(target) => {
                    let take_if = matches!(op, Op::JumpTrue(_));
                    let Value::Bool(condition) = self.pop() else {
                        unreachable!("typed branch");
                    };
                    if condition == take_if {
                        pc = *target;
                    }
                    continue;
                }
                Op::Return => {
                    let value = self.pop();
                    let frame = self.frames.pop().expect("active frame");
                    self.locals.truncate(frame.slots);
                    self.operands.truncate(frame.stack);
                    let Some(caller) = self.frames.last() else {
                        return Ok(value);
                    };
                    (ops, slots, pc) = activate(caller);
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
        (Unary::I64ToF32, Value::I64(value)) => Value::F32((value as f32).to_bits()),
        (Unary::F32ToI64, Value::F32(bits)) => Value::I64(f32::from_bits(bits) as i64),
        (Unary::SqrtF32, Value::F32(bits)) => {
            let value = f32::from_bits(bits).sqrt();
            Value::F32(if value.is_nan() {
                0x7fc0_0000
            } else {
                value.to_bits()
            })
        }
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
                And => Ok($variant($left & $right)),
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
