use super::compile::{Code, Instruction, Op, Operand};
use super::*;

impl Module {
    /// Resolve one export and prepare its available native tier once. The
    /// returned handle borrows this exact immutable module revision; it cannot
    /// be rebound to a replacement module or to a caller-supplied export index.
    pub fn prepare_call(&self, export: &str) -> Result<PreparedCall<'_>, Error> {
        let (export, &function) = self
            .exports
            .get_key_value(export)
            .ok_or_else(|| error(format!("unknown export {export}")))?;
        self.prepare_function(function);
        Ok(PreparedCall {
            module: self,
            export,
            function,
            code: &self.code[function],
            #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
            scalar: self.code[function].native.get().and_then(Option::as_ref),
            #[cfg(all(
                feature = "typed-jit",
                target_arch = "x86_64",
                any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
            ))]
            typed: self.native[function].get().and_then(Option::as_ref),
        })
    }

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

    /// Execute the same checked VM with an explicit native-service registry.
    /// Effectful host opcodes always remain on this path: native fallback must
    /// never replay a side effect after a partially executed call.
    pub fn run_with_host(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
        host: &dyn HostServices,
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
        let result =
            self.evaluate_portable_with_host(invocation, workspace, function, code, Some(host));
        workspace.operands.clear();
        workspace.locals.clear();
        workspace.frames.clear();
        result
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
        #[cfg(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        ))]
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
        #[cfg(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        ))]
        return self.native[_function]
            .get_or_init(|| super::typed_jit::Native::compile(self, _function))
            .is_some();
        #[cfg(not(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        )))]
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
        #[cfg(any(
            all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"),
            all(
                feature = "typed-jit",
                target_os = "linux",
                target_env = "gnu",
                target_arch = "x86_64"
            )
        ))]
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
        self.evaluate_portable_with_host(invocation, workspace, function, code, None)
    }

    fn evaluate_portable_with_host(
        &self,
        invocation: Invocation<'_>,
        workspace: &mut Workspace,
        function: usize,
        code: &Code,
        host: Option<&dyn HostServices>,
    ) -> Result<Value, Error> {
        let mut machine = Machine {
            module: self,
            host,
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

/// The execution route that produced an invocation's result. A prepared native
/// function is not proof of a hit: declined invocations report `Portable` after
/// replaying the unchanged evaluator. `PrecompiledScan` is not generated code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionRoute {
    Portable,
    AheadOfTime,
    NativeScalar,
    TypedJit,
    PrecompiledScan,
}

/// A pre-resolved export belonging to one immutable module. Shared calls use
/// caller-owned workspaces and invocation-local scratch; neither generated code
/// nor the handle retain argument pointers. Dropping a different module revision
/// cannot invalidate this handle. It must not outlive its borrowed module.
pub struct PreparedCall<'module> {
    module: &'module Module,
    export: &'module str,
    function: usize,
    code: &'module Code,
    #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
    scalar: Option<&'module super::native_scalar::Scalar>,
    #[cfg(all(
        feature = "typed-jit",
        target_arch = "x86_64",
        any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
    ))]
    typed: Option<&'module super::typed_jit::Native>,
}

impl PreparedCall<'_> {
    /// The pre-resolved checked export with an explicit native service table.
    /// This path never speculates into a native tier before executing effects.
    pub fn run_with_host(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
        host: &dyn HostServices,
    ) -> Result<Value, Error> {
        if inputs.len() != self.module.interface.inputs.len()
            || arguments.len() != self.code.params.len()
        {
            return Err(error("host argument or input count differs from interface"));
        }
        let result = self.module.evaluate_portable_with_host(
            Invocation {
                export: self.export,
                inputs,
                arguments,
                limits,
            },
            workspace,
            self.function,
            self.code,
            Some(host),
        );
        workspace.operands.clear();
        workspace.locals.clear();
        workspace.frames.clear();
        result
    }

    #[inline]
    pub fn run(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
    ) -> Result<Value, Error> {
        self.run_with(inputs, arguments, limits, &mut Workspace::default())
    }

    #[inline]
    pub fn run_with(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        self.run_with_route(inputs, arguments, limits, workspace).0
    }

    /// Return actual route attribution alongside both successful and failed
    /// results, without mutable counters or a global telemetry dependency.
    pub fn run_with_route(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
    ) -> (Result<Value, Error>, ExecutionRoute) {
        if inputs.len() != self.module.interface.inputs.len()
            || arguments.len() != self.code.params.len()
        {
            return (
                Err(error("host argument or input count differs from interface")),
                ExecutionRoute::Portable,
            );
        }
        let invocation = Invocation {
            export: self.export,
            inputs,
            arguments,
            limits,
        };
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        if let Some(Ok(value)) = self.scalar.and_then(|native| native.run(&invocation)) {
            return (Ok(value), ExecutionRoute::NativeScalar);
        }
        #[cfg(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        ))]
        if let Some(native) = self.typed
            && let Some(value) = native.run(&invocation)
        {
            return (Ok(value), native.route());
        }
        // A speculative failure performs no externally visible effects. Replay
        // preserves the original admission/evaluation/first-trap schedule. No
        // new compiler work occurs on this prepared invocation path.
        (
            self.module.run_portable_function(
                invocation,
                workspace,
                self.function,
                self.code,
                false,
            ),
            ExecutionRoute::Portable,
        )
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
    host: Option<&'a dyn HostServices>,
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
    checked_min_f32_index(values).expect("typed float")
}

/// The speculative scan validates tags while computing the same strict minimum
/// as the portable evaluator. Its caller must reserve all resource charges
/// before entering this pure loop; a bad tag declines for portable error replay.
#[inline]
pub(super) fn checked_min_f32_index(values: &[Value]) -> Option<u64> {
    let Some((first, rest)) = values.split_first() else {
        return Some(0);
    };
    let Value::F32(bits) = first else {
        return None;
    };
    let magnitude = *bits & 0x7fff_ffff;
    if magnitude > 0x7f80_0000 {
        // A first NaN remains selected, but all later tags must still be checked.
        return rest
            .iter()
            .all(|value| matches!(value, Value::F32(_)))
            .then_some(0);
    }
    let mut best = if magnitude == 0 {
        0x8000_0000
    } else {
        raw_f32_key(*bits)
    };
    let mut selected = 0;
    let mut pairs = rest.chunks_exact(2);
    let mut index = 1;
    for pair in &mut pairs {
        let [Value::F32(left), Value::F32(right)] = pair else {
            return None;
        };
        let left_key = raw_f32_key(*left);
        let right_key = raw_f32_key(*right);
        // Computing both keys exposes independent work while selection stays
        // in source order, retaining the first equal value in either lane.
        consider_f32_minimum(*left, left_key, index, &mut best, &mut selected);
        consider_f32_minimum(*right, right_key, index + 1, &mut best, &mut selected);
        index += 2;
    }
    if let [last] = pairs.remainder() {
        let Value::F32(bits) = last else {
            return None;
        };
        consider_f32_minimum(*bits, raw_f32_key(*bits), index, &mut best, &mut selected);
    }
    Some(selected as u64)
}

#[inline]
fn raw_f32_key(bits: u32) -> u32 {
    bits ^ (0u32.wrapping_sub(bits >> 31) | 0x8000_0000)
}

#[inline(always)]
fn consider_f32_minimum(bits: u32, key: u32, index: usize, best: &mut u32, selected: &mut usize) {
    if key < *best {
        let magnitude = bits & 0x7fff_ffff;
        // Best is always ordered, with either zero normalized to 0x80000000.
        // Positive NaNs cannot win; negative NaNs are rejected here. A winning
        // zero remains ordered after normalization, but must not replace zero.
        if magnitude <= 0x7f80_0000 && (magnitude != 0 || *best != 0x8000_0000) {
            *best = if magnitude == 0 { 0x8000_0000 } else { key };
            *selected = index;
        }
    }
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
                Op::RawRuleBinary(kind, left, right) => {
                    let (Value::I64(left), Value::I64(right)) =
                        (self.operand(left, slots)?, self.operand(right, slots)?)
                    else {
                        unreachable!("checked raw rule operands");
                    };
                    Value::I64(
                        kind.evaluate(*left, *right)
                            .expect("checked raw rule operator"),
                    )
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
                Op::EmptyLocal(index) | Op::LastLocal(index) => {
                    // The load's fuel was charged above. Resolve its local
                    // before the second charge, so uninitialized reads keep
                    // precedence over an exhausted list-operation budget.
                    let value = self.locals[slots + index]
                        .as_ref()
                        .ok_or_else(|| error("read of uninitialized local"))?;
                    let Value::List(values) = value else {
                        unreachable!("typed list");
                    };
                    debug_assert!(matches!(
                        (op, &ops[pc].op),
                        (Op::EmptyLocal(_), Op::Empty) | (Op::LastLocal(_), Op::Last)
                    ));
                    // Both original instruction slots and charges survive.
                    // Borrow the local across this disjoint fuel update;
                    // no temporary list Arc or operand-stack entry is needed.
                    self.remaining.instructions = self
                        .remaining
                        .instructions
                        .checked_sub(ops[pc].fuel)
                        .ok_or_else(|| error("module instruction budget exhausted"))?;
                    pc += 1;
                    if matches!(op, Op::EmptyLocal(_)) {
                        Value::Bool(values.is_empty())
                    } else {
                        values
                            .last()
                            .cloned()
                            .ok_or_else(|| error("last from empty list"))?
                    }
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
                Op::HostCall { name, signature } => {
                    let host = self
                        .host
                        .ok_or_else(|| error("native host service registry is unavailable"))?;
                    let arguments = self
                        .operands
                        .split_off(self.operands.len() - signature.params.len());
                    // Calls are typed by the common checker. Check again at
                    // the native boundary and charge returned object graphs.
                    for (value, ty) in arguments.iter().zip(&signature.params) {
                        self.check_value(value, ty, 0)?;
                    }
                    let value = host.call(name, &arguments)?;
                    self.check_value(&value, &signature.result, 0)?;
                    value
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

#[cfg(test)]
mod borrowed_list_read_tests {
    use super::*;

    fn list(values: &[i64]) -> Value {
        Value::List(Arc::new(values.iter().copied().map(Value::I64).collect()))
    }

    // Reconstruct the original two independently dispatched instructions,
    // without using the borrowed handler as the expected-value implementation.
    fn unfused(source: &str) -> Module {
        let mut module = Module::compile_compact(source).unwrap();
        let mut changed = 0;
        for code in &mut module.code {
            let code = Arc::get_mut(code).expect("fresh unshared code");
            let mut consumed = Vec::new();
            for pc in 0..code.ops.len() {
                let index = match &code.ops[pc].op {
                    Op::EmptyLocal(index) => {
                        assert!(matches!(code.ops[pc + 1].op, Op::Empty));
                        *index
                    }
                    Op::LastLocal(index) => {
                        assert!(matches!(code.ops[pc + 1].op, Op::Last));
                        *index
                    }
                    _ => continue,
                };
                assert_eq!(code.ops[pc].fuel, 1);
                assert_eq!(code.ops[pc + 1].fuel, 1);
                code.ops[pc].op = Op::Operand(Operand::Local(index));
                consumed.push(pc + 1);
                changed += 1;
            }
            for instruction in &code.ops {
                if let Op::Jump(to) | Op::JumpTrue(to) | Op::JumpFalse(to) = &instruction.op {
                    assert!(!consumed.contains(to), "branch enters a consumed slot");
                }
            }
        }
        assert!(changed > 0, "oracle fixture must exercise borrowed reads");
        module
    }

    fn compare_grid(source: &str, arguments: &[Value], maximum_fuel: u64) {
        let optimized = Module::compile_compact(source).unwrap();
        let reference = unfused(source);
        assert_eq!(optimized.encode(), reference.encode());
        for (before, after) in reference.code.iter().zip(&optimized.code) {
            assert_eq!(before.ops.len(), after.ops.len());
            assert_eq!(
                before.ops.iter().map(|op| op.fuel).collect::<Vec<_>>(),
                after.ops.iter().map(|op| op.fuel).collect::<Vec<_>>()
            );
        }
        let original = arguments.to_vec();
        let mut actual_ws = Workspace::default();
        let mut reference_ws = Workspace::default();
        for instructions in 0..=maximum_fuel {
            for cells in 0..=32 {
                for frames in 0..=2 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    let invoke = || Invocation {
                        export: "run",
                        inputs: &[],
                        arguments,
                        limits,
                    };
                    let actual = optimized
                        .run_portable_with(invoke(), &mut actual_ws)
                        .map_err(|error| error.to_string());
                    let expected = reference
                        .run_portable_with(invoke(), &mut reference_ws)
                        .map_err(|error| error.to_string());
                    assert_eq!(actual, expected, "{source}: {limits:?}");
                    for workspace in [&actual_ws, &reference_ws] {
                        assert!(workspace.operands.is_empty());
                        assert!(workspace.locals.is_empty());
                        assert!(workspace.frames.is_empty());
                    }
                    assert_eq!(arguments, original);
                }
            }
        }
    }

    #[test]
    fn borrowed_reads_match_unfused_instructions_at_every_resource_boundary() {
        for source in [
            "v2 (pub run (xs:[i64]) bool = (empty xs))",
            "v2 (pub run (xs:[i64]) i64 = (last xs))",
        ] {
            for argument in [list(&[]), list(&[3, 7]), Value::Bool(false)] {
                compare_grid(source, &[argument], 5);
            }
            compare_grid(
                source,
                &[Value::List(Arc::new(vec![
                    Value::I64(3),
                    Value::Bool(false),
                ]))],
                5,
            );
        }
        for source in [
            "v2 (pub run () bool (xs:[i64]) (ret (empty xs)))",
            "v2 (pub run () i64 (xs:[i64]) (ret (last xs)))",
        ] {
            compare_grid(source, &[], 5);
        }
    }

    #[test]
    fn borrowed_reads_keep_loops_calls_copy_on_write_and_trap_cleanup() {
        let loop_source = "v2 (pub run (xs:[i64]) i64 (sum:i64 top:i64)
            (= sum 0)
            (while (! (empty xs)) (
                (= top (last xs)) (pop xs) (= sum (+ sum top))))
            (ret sum))";
        for values in [&[][..], &[3, 7, 11][..], &[1, i64::MAX][..]] {
            compare_grid(loop_source, &[list(values)], 64);
        }
        compare_grid(
            "v2 (fn tail (xs:[i64]) i64 = (last xs))
                (pub run (xs:[i64]) i64 = (call tail xs))",
            &[list(&[3, 7])],
            10,
        );
        compare_grid(
            "v2 (pub run (xs:[i64] x:i64) [i64] ()
                (if (! (empty xs)) ((pop xs)))
                (push xs x) (push xs (last xs)) (ret xs))",
            &[list(&[3, 7, 11]), Value::I64(-4)],
            24,
        );
    }
}
