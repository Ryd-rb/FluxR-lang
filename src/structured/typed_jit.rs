//! Native execution for the shared typed subset on Win64 and Linux GNU x86-64.
use super::compile::Operand;
#[cfg(target_os = "linux")]
use super::linux_jit::RegisteredPage;
use super::native_ir::{
    Boundary, Exit, Function, MAX_LANES, Plan, ScalarAdmission, Shape, Step, TaggedScalarAdmission,
};
use super::vm::{checked_min_f32_index, scan_effect};
use super::{Binary, Invocation, Module, Type, Value};
#[cfg(target_os = "windows")]
use crate::rules::native_jit::RegisteredPage;
use cranelift_codegen::ir::{self, Block, InstBuilder, Value as Register, condcodes::IntCC, types};
use cranelift_codegen::isa::{self, CallConv, TargetFrontendConfig, TargetIsa, unwind::UnwindInfo};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, control::ControlPlane};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use std::sync::{Arc, OnceLock};

pub(super) type Cache = OnceLock<Option<Native>>;
const SMALL_LANES: usize = 4;

#[derive(Debug)]
pub(super) enum Native {
    Scalar(ScalarJit),
    TaggedScalar(TaggedScalarJit),
    Small(Jit<SMALL_LANES>),
    Wide(Jit<MAX_LANES>),
    Scan(Scan),
}

impl Native {
    pub(super) fn compile(module: &Module, index: usize) -> Option<Self> {
        if let Some(scan) = Scan::lower(module, index) {
            return Some(Self::Scan(scan));
        }
        Self::compile_jit(module, index)
    }

    #[inline]
    pub(super) fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        match self {
            Self::Scalar(jit) => jit.run(invocation),
            Self::TaggedScalar(jit) => jit.run(invocation),
            Self::Small(jit) => jit.run(invocation),
            Self::Wide(jit) => jit.run(invocation),
            Self::Scan(scan) => scan.run(invocation),
        }
    }

    pub(super) fn route(&self) -> super::ExecutionRoute {
        match self {
            Self::Scalar(_) | Self::TaggedScalar(_) | Self::Small(_) | Self::Wide(_) => {
                super::ExecutionRoute::TypedJit
            }
            Self::Scan(_) => super::ExecutionRoute::PrecompiledScan,
        }
    }
}

/// A precompiled kernel over an admitted list borrowed from the host.
#[derive(Debug)]
pub(super) struct Scan {
    input: bool,
    fixed_fuel: u64,
    fixed_cells: usize,
}

impl Scan {
    fn lower(module: &Module, index: usize) -> Option<Self> {
        let code = &module.code[index];
        let (load, scan_fuel, return_fuel) = scan_effect(&code.ops)?;
        let (input, ty) = match (load, &module.interface.inputs[..], &code.params[..]) {
            (Operand::Input(0), [binding], []) => (true, &binding.ty),
            (Operand::Local(0), [], [ty]) => (false, ty),
            _ => return None,
        };
        if !matches!(ty, Type::List(element) if **element == Type::F32) || code.result != Type::U64
        {
            return None;
        }
        Some(Self {
            input,
            fixed_fuel: scan_fuel + return_fuel,
            fixed_cells: code.slots + 3, // List wrapper, frame and result.
        })
    }

    #[inline(never)]
    fn run(&self, call: &Invocation<'_>) -> Option<Value> {
        let list = if self.input {
            call.inputs
        } else {
            call.arguments
        };
        let [Value::List(values)] = list else {
            return None;
        };
        // Each rejection replays the portable evaluator for exact error order.
        // Every successful call has reserved full scan fuel before evaluation.
        if call.limits.frames == 0
            || call.limits.cells.checked_sub(self.fixed_cells)? < values.len()
            || call.limits.instructions.checked_sub(self.fixed_fuel)? < values.len() as u64
        {
            return None;
        }
        checked_min_f32_index(values).map(Value::U64)
    }
}

#[derive(Debug)]
pub(super) struct Jit<const LANES: usize> {
    boundary: Boundary,
    page: RegisteredPage,
}

/// The admission and code owner for the fixed five-argument scalar ABI. A
/// scalar entry does not retain the generic boundary it no longer uses.
#[derive(Debug)]
pub(super) struct ScalarJit {
    admission: ScalarAdmission,
    page: RegisteredPage,
}

/// Fixed-bound Boolean/signed/unsigned scalar calls have their own packing
/// path, so signed-only calls do not acquire per-lane tag discrimination.
#[derive(Debug)]
pub(super) struct TaggedScalarJit {
    admission: TaggedScalarAdmission,
    page: RegisteredPage,
}

impl Native {
    fn compile_jit(module: &Module, index: usize) -> Option<Self> {
        let mut plan = Plan::lower(module, index)?;
        if plan.boundary.fixed.is_none() {
            // This stronger admission belongs to the process-owned JIT only.
            // Keep the shared Plan/AOT contract and generated AOT bytes intact.
            plan.boundary.fixed = acyclic_bound(&plan);
        }
        #[cfg(target_os = "windows")]
        let call_conv = CallConv::WindowsFastcall;
        #[cfg(target_os = "linux")]
        let call_conv = CallConv::SystemV;
        let scalar = plan.boundary.scalar_admission();
        let tagged_scalar = plan.boundary.tagged_scalar_admission();
        let scalar_abi = scalar.is_some() || tagged_scalar.is_some();
        let mut signature = ir::Signature::new(call_conv);
        // Scalar entry: four value registers (unused values are zero), then
        // one result pointer. Generic entry: input/output pointers, fuel, cells.
        // A single fixed scalar signature avoids arity-dependent transmutation.
        signature.params.extend(std::iter::repeat_n(
            ir::AbiParam::new(types::I64),
            if scalar_abi { 5 } else { 4 },
        ));
        signature.returns.push(ir::AbiParam::new(types::I32));
        let mut context = Context::for_function(ir::Function::with_name_signature(
            ir::UserFuncName::default(),
            signature,
        ));
        let mut frontend = FunctionBuilderContext::new();
        let target = target()?;
        emit(
            FunctionBuilder::new(&mut context.func, &mut frontend),
            &plan,
            target.frontend_config(),
            scalar_abi,
        );
        let code = context.compile(target, &mut ControlPlane::default()).ok()?;
        // Calls (including outline stack probes) require external relocation.
        // The only allowed trap metadata belongs to explicitly guarded division.
        if !code.buffer.relocs().is_empty()
            || code.buffer.traps().iter().any(|trap| {
                !matches!(
                    trap.code,
                    ir::TrapCode::INTEGER_OVERFLOW | ir::TrapCode::INTEGER_DIVISION_BY_ZERO
                )
            })
        {
            return None;
        }
        #[cfg(target_os = "windows")]
        let page = {
            let UnwindInfo::WindowsX64(info) = code.create_unwind_info(target).ok()?? else {
                return None;
            };
            let mut unwind = vec![0; info.emit_size()];
            info.emit(&mut unwind);
            RegisteredPage::publish(code.code_buffer(), &unwind).ok()?
        };
        #[cfg(target_os = "linux")]
        let page = {
            let UnwindInfo::SystemV(info) = code.create_unwind_info(target).ok()?? else {
                return None;
            };
            RegisteredPage::publish(code.code_buffer(), info, target).ok()?
        };
        if let Some(admission) = scalar {
            return Some(Self::Scalar(ScalarJit { admission, page }));
        }
        if let Some(admission) = tagged_scalar {
            return Some(Self::TaggedScalar(TaggedScalarJit { admission, page }));
        }
        let boundary = plan.boundary;
        Some(if boundary.fits_lanes(SMALL_LANES) {
            Self::Small(Jit { boundary, page })
        } else {
            Self::Wide(Jit { boundary, page })
        })
    }
}

impl ScalarJit {
    // Keep scalar scratch in this entry, outside generic and scan dispatch.
    #[inline(never)]
    fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        let values = self.admission.pack(invocation)?;
        let mut result = 0i64;
        type Entry = unsafe extern "system" fn(i64, i64, i64, i64, *mut i64) -> u32;
        // SAFETY: scalar admission and this exact five-argument signature
        // were selected together when emitting the owned code. Admission
        // has checked every tag, both arities and the complete fuel/cell/
        // frame ceiling. The generated kernel reads scalar arguments only
        // and writes at most one i64 to this live, aligned result slot.
        // Arithmetic guards still decline before an invalid operation.
        // The platform ABI and registered unwind metadata are unchanged;
        // on Win64 the fifth argument follows that ABI's stack convention.
        let status = unsafe {
            let entry: Entry = std::mem::transmute(self.page.entry());
            entry(values[0], values[1], values[2], values[3], &mut result)
        };
        (status == 0).then_some(Value::I64(result))
    }
}

impl TaggedScalarJit {
    #[inline(never)]
    fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        let values = self.admission.pack(invocation)?;
        let mut result = 0i64;
        type Entry = unsafe extern "system" fn(i64, i64, i64, i64, *mut i64) -> u32;
        // SAFETY: This owner was emitted with the same fixed scalar signature.
        // Admission checks exact counts, every declared I64/U64/Bool tag and the
        // complete resource ceiling before supplying four initialized lanes.
        // Boolean values are canonical 0/1; unsigned lanes preserve all 64 bits.
        // The checked Plan fixes the result tag. Generated code writes one i64
        // to this aligned live result slot.
        // Its immutable RX owner and registered platform unwind metadata remain
        // borrowed throughout the call. A decline replays the portable oracle.
        let status = unsafe {
            let entry: Entry = std::mem::transmute(self.page.entry());
            entry(values[0], values[1], values[2], values[3], &mut result)
        };
        (status == 0).then(|| self.admission.unpack(result))
    }
}

impl<const LANES: usize> Jit<LANES> {
    #[inline]
    fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        debug_assert!(self.boundary.fits_lanes(LANES));
        self.boundary
            .run::<LANES>(invocation, |input, output, fuel, cells| {
                type Entry = unsafe extern "system" fn(*const i64, *mut i64, u64, u64) -> u32;
                // SAFETY: The immutable RX owner outlives this call. The only memory
                // operations are the input loads and result stores emitted for the
                // same boundary that selected LANES.
                // Arithmetic guards prevent division faults. The platform owner
                // supplies registered Windows or System V unwind metadata. On
                // Linux x86-64, extern "system" uses the emitted System V C ABI.
                unsafe {
                    let entry: Entry = std::mem::transmute(self.page.entry());
                    entry(input.as_ptr(), output.as_mut_ptr(), fuel, cells)
                }
            })
    }
}

type ResourceBound = (u64, usize);

#[derive(Clone, Copy)]
enum BoundState {
    Unvisited,
    Active,
    Complete(Option<ResourceBound>),
}

/// A sufficient fuel/cell reservation for every reachable path through an
/// acyclic Plan. Calls contribute at each occurrence, while joins and callees
/// are analyzed once. The checked Plan's expanded-operation and call-depth
/// limits also bound this analysis; a cycle or overflowing bound keeps the
/// original dynamic meters. Lower budgets replay the portable evaluator.
fn acyclic_bound(plan: &Plan) -> Option<ResourceBound> {
    BoundAnalysis {
        plan,
        functions: vec![BoundState::Unvisited; plan.functions.len()],
    }
    .function(plan.entry)
}

struct BoundAnalysis<'plan> {
    plan: &'plan Plan,
    functions: Vec<BoundState>,
}

impl BoundAnalysis<'_> {
    fn function(&mut self, index: usize) -> Option<ResourceBound> {
        match self.functions.get(index)? {
            BoundState::Active => return None,
            BoundState::Complete(bound) => return *bound,
            BoundState::Unvisited => {}
        }
        self.functions[index] = BoundState::Active;
        let function = self.plan.functions.get(index)?.as_ref()?;
        let mut blocks = vec![BoundState::Unvisited; function.blocks.len()];
        let bound = self.block(function, 0, &mut blocks);
        self.functions[index] = BoundState::Complete(bound);
        bound
    }

    fn block(
        &mut self,
        function: &Function,
        index: usize,
        states: &mut [BoundState],
    ) -> Option<ResourceBound> {
        match states.get(index)? {
            BoundState::Active => return None,
            BoundState::Complete(bound) => return *bound,
            BoundState::Unvisited => {}
        }
        states[index] = BoundState::Active;
        let block = function.blocks.get(index)?;
        let mut bound = (block.fuel, block.cells);
        for step in &block.steps {
            if let Step::Call(callee, _) = step {
                bound = add_bound(bound, self.function(*callee)?)?;
            }
        }
        let tail = match block.exit {
            Exit::Return(_) => (0, 0),
            Exit::Jump(next) => self.block(function, next, states)?,
            Exit::Branch(_, yes, no) => {
                let yes = self.block(function, yes, states)?;
                let no = self.block(function, no, states)?;
                // The paths with the most fuel and cells need not coincide.
                (yes.0.max(no.0), yes.1.max(no.1))
            }
        };
        let bound = add_bound(bound, tail);
        states[index] = BoundState::Complete(bound);
        bound
    }
}

fn add_bound(left: ResourceBound, right: ResourceBound) -> Option<ResourceBound> {
    Some((left.0.checked_add(right.0)?, left.1.checked_add(right.1)?))
}

/// Move exact block charges to the start of straight-line regions. Conditional
/// successors and the function entry start new regions; pure jump cycles also
/// retain a metered entry. A shared unconditional suffix may belong to several
/// regions, but an executed path pays for it exactly once. No maximum across
/// conditional alternatives is used, so a loop still succeeds natively at its
/// original exact budget. Callees keep their own independent region charges.
///
/// This is an emission-only schedule over the checked Plan. An early trap or a
/// failed reservation still declines to the portable evaluator, preserving its
/// first error. Source instructions, shared AOT metering and artifact bytes are
/// untouched. All traversals and allocations are bounded by the Plan size.
fn linear_region_meters(function: &Function) -> Option<Vec<ResourceBound>> {
    let count = function.blocks.len();
    if count == 0 {
        return None;
    }
    let mut starts = vec![false; count];
    starts[0] = true;
    for block in &function.blocks {
        match block.exit {
            Exit::Branch(_, yes, no) => {
                *starts.get_mut(yes)? = true;
                *starts.get_mut(no)? = true;
            }
            Exit::Jump(next) if next >= count => return None,
            _ => {}
        }
    }

    // Cutting conditional edges already breaks ordinary loop cycles. Mark
    // only cycles left in unconditional chains; marking every CFG backedge
    // would unnecessarily split a loop body from its following condition.
    fn cut_jumps(function: &Function, index: usize, starts: &mut [bool], seen: &mut [u8]) {
        if seen[index] != 0 {
            return;
        }
        seen[index] = 1;
        if let Exit::Jump(next) = function.blocks[index].exit
            && !starts[next]
        {
            if seen[next] == 1 {
                starts[next] = true;
            } else {
                cut_jumps(function, next, starts, seen);
            }
        }
        seen[index] = 2;
    }
    let mut seen = vec![0; count];
    for index in 0..count {
        cut_jumps(function, index, &mut starts, &mut seen);
    }

    fn bound(
        function: &Function,
        index: usize,
        starts: &[bool],
        states: &mut [BoundState],
    ) -> Option<ResourceBound> {
        match states[index] {
            BoundState::Active => return None,
            BoundState::Complete(value) => return value,
            BoundState::Unvisited => {}
        }
        states[index] = BoundState::Active;
        let block = &function.blocks[index];
        let tail = match block.exit {
            Exit::Jump(next) if !starts[next] => bound(function, next, starts, states)?,
            _ => (0, 0),
        };
        let value = add_bound((block.fuel, block.cells), tail);
        states[index] = BoundState::Complete(value);
        value
    }
    let mut states = vec![BoundState::Unvisited; count];
    let mut meters = vec![(0, 0); count];
    for index in 0..count {
        if starts[index] {
            meters[index] = bound(function, index, &starts, &mut states)?;
        }
    }
    Some(meters)
}

fn target() -> Option<&'static dyn TargetIsa> {
    static ISA: OnceLock<Option<Arc<dyn TargetIsa>>> = OnceLock::new();
    ISA.get_or_init(|| {
        let mut flags = settings::builder();
        flags.set("opt_level", "speed").ok()?;
        flags.set("enable_verifier", "true").ok()?;
        flags.set("enable_probestack", "true").ok()?;
        #[cfg(target_os = "windows")]
        let triple = "x86_64-pc-windows-msvc";
        #[cfg(target_os = "linux")]
        let triple = "x86_64-unknown-linux-gnu";
        isa::lookup_by_name(triple)
            .ok()?
            .finish(settings::Flags::new(flags))
            .ok()
    })
    .as_deref()
}

fn emit(mut builder: FunctionBuilder<'_>, plan: &Plan, config: TargetFrontendConfig, scalar: bool) {
    let start = builder.create_block();
    builder.append_block_params_for_function_params(start);
    builder.switch_to_block(start);
    let abi = builder.block_params(start).to_vec();
    let fail = builder.create_block();
    let mut emit = Emitter {
        builder,
        plan,
        fail,
        remaining: None,
        inputs: Vec::new(),
    };
    if plan.boundary.fixed.is_none() {
        let fuel = emit.variable(abi[2]);
        let cells = emit.variable(abi[3]);
        emit.remaining = Some([fuel, cells]);
    }
    let (args, output) = if scalar {
        debug_assert!(
            plan.boundary.scalar_admission().is_some()
                || plan.boundary.tagged_scalar_admission().is_some()
        );
        let inputs = plan.boundary.inputs.len();
        let end = inputs + plan.boundary.args.len();
        emit.inputs = abi[..inputs].iter().map(|&value| vec![value]).collect();
        let args = abi[inputs..end].iter().map(|&value| vec![value]).collect();
        (args, abi[4])
    } else {
        let mut offset = 0;
        let mut load = |shape: &Shape| {
            (0..shape.lanes())
                .map(|_| {
                    let reg = emit.builder.ins().load(
                        types::I64,
                        ir::MemFlagsData::trusted(),
                        abi[0],
                        offset,
                    );
                    offset += 8;
                    reg
                })
                .collect::<Vec<_>>()
        };
        emit.inputs = plan.boundary.inputs.iter().map(&mut load).collect();
        let args = plan.boundary.args.iter().map(&mut load).collect();
        (args, abi[1])
    };
    let results = emit.function(plan.function(plan.entry), args);
    for (lane, result) in results.into_iter().enumerate() {
        emit.builder.ins().store(
            ir::MemFlagsData::trusted(),
            result,
            output,
            (lane * 8) as i32,
        );
    }
    let ok = emit.builder.ins().iconst(types::I32, 0);
    emit.builder.ins().return_(&[ok]);
    emit.builder.switch_to_block(fail);
    let failed = emit.builder.ins().iconst(types::I32, 1);
    emit.builder.ins().return_(&[failed]);
    emit.builder.seal_all_blocks();
    emit.builder.finalize(config);
}

struct Emitter<'a, 'plan> {
    builder: FunctionBuilder<'a>,
    plan: &'plan Plan,
    fail: Block,
    remaining: Option<[Variable; 2]>,
    inputs: Vec<Vec<Register>>,
}

struct Local {
    lanes: Vec<Variable>,
    initialized: Option<Variable>,
}

/// Determine which locals still require runtime initialization state. This is
/// a bounded must analysis over the already checked native Plan, not a new
/// source-language definite-assignment rule: uncertain reads retain the old
/// dynamic guard and can still replay the released portable evaluator.
fn initialization_guards(function: &Function) -> Vec<bool> {
    let count = function.blocks.len();
    let slots = function.slots.len();
    let mut predecessors = vec![Vec::new(); count];
    let mut reachable = vec![false; count];
    let mut pending = vec![0];
    while let Some(index) = pending.pop() {
        if reachable[index] {
            continue;
        }
        reachable[index] = true;
        match function.blocks[index].exit {
            Exit::Jump(next) => pending.push(next),
            Exit::Branch(_, yes, no) => pending.extend([yes, no]),
            Exit::Return(_) => {}
        }
    }
    for (index, block) in function.blocks.iter().enumerate() {
        if !reachable[index] {
            continue;
        }
        let mut edge = |next: usize| predecessors[next].push(index);
        match block.exit {
            Exit::Jump(next) => edge(next),
            Exit::Branch(_, yes, no) => {
                edge(yes);
                edge(no);
            }
            Exit::Return(_) => {}
        }
    }
    let entry: Vec<_> = (0..slots).map(|slot| slot < function.params).collect();
    let mut incoming = vec![vec![true; slots]; count];
    let mut outgoing = incoming.clone();
    // Start at the lattice top and only remove facts. Each bit can change at
    // most once; a maximum of 512 expanded operations and 32 lanes bounds this
    // work. The entry's initial edge participates even if a loop targets it.
    loop {
        let mut changed = false;
        for (index, block) in function.blocks.iter().enumerate() {
            let mut before = if reachable[index] {
                if index == 0 {
                    entry.clone()
                } else {
                    vec![true; slots]
                }
            } else {
                vec![false; slots]
            };
            for &predecessor in &predecessors[index] {
                for (known, &other) in before.iter_mut().zip(&outgoing[predecessor]) {
                    *known &= other;
                }
            }
            let mut after = before.clone();
            for step in &block.steps {
                if let Step::Store(slot, _) = *step {
                    after[slot] = true;
                }
            }
            changed |= incoming[index] != before || outgoing[index] != after;
            incoming[index] = before;
            outgoing[index] = after;
        }
        if !changed {
            break;
        }
    }
    let mut guarded = vec![false; slots];
    for (index, block) in function.blocks.iter().enumerate() {
        let mut known = incoming[index].clone();
        for step in &block.steps {
            match *step {
                Step::Load(slot) | Step::Update(slot, _, _) => guarded[slot] |= !known[slot],
                Step::Store(slot, _) => known[slot] = true,
                _ => {}
            }
        }
    }
    guarded
}

impl Emitter<'_, '_> {
    fn constant(&mut self, value: i64) -> Register {
        self.builder.ins().iconst(types::I64, value)
    }

    fn variable(&mut self, value: Register) -> Variable {
        let var = self.builder.declare_var(types::I64);
        self.builder.def_var(var, value);
        var
    }

    fn decline_if(&mut self, condition: Register) {
        let next = self.builder.create_block();
        self.builder
            .ins()
            .brif(condition, self.fail, &[], next, &[]);
        self.builder.switch_to_block(next);
    }

    fn meter(&mut self, fuel: u64, cells: usize) {
        let Some(remaining) = self.remaining else {
            return;
        };
        for (var, amount) in remaining.into_iter().zip([fuel as i64, cells as i64]) {
            if amount > 0 {
                let left = self.builder.use_var(var);
                let missing = self
                    .builder
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThan, left, amount);
                self.decline_if(missing);
                let left = self.builder.ins().iadd_imm_s(left, -amount);
                self.builder.def_var(var, left);
            }
        }
    }

    fn function(&mut self, function: &Function, args: Vec<Vec<Register>>) -> Vec<Register> {
        let zero = self.constant(0);
        let guarded = initialization_guards(function);
        let meters = self.remaining.and_then(|_| linear_region_meters(function));
        let locals: Vec<_> = function
            .slots
            .iter()
            .enumerate()
            .map(|(index, shape)| Local {
                lanes: (0..shape.lanes())
                    .map(|lane| self.variable(args.get(index).map_or(zero, |arg| arg[lane])))
                    .collect(),
                initialized: (index >= function.params && guarded[index])
                    .then(|| self.variable(zero)),
            })
            .collect();
        let blocks: Vec<_> = function
            .blocks
            .iter()
            .map(|_| self.builder.create_block())
            .collect();
        let done = self.builder.create_block();
        for _ in 0..function.result.lanes() {
            self.builder.append_block_param(done, types::I64);
        }
        self.builder.ins().jump(blocks[0], &[]);
        for (index, (block, &label)) in function.blocks.iter().zip(&blocks).enumerate() {
            self.builder.switch_to_block(label);
            let (fuel, cells) = meters
                .as_ref()
                .map_or((block.fuel, block.cells), |meters| meters[index]);
            self.meter(fuel, cells);
            let mut temps: Vec<Vec<Register>> = Vec::with_capacity(block.steps.len());
            for step in &block.steps {
                let value = match step {
                    Step::Const(value) => vec![self.constant(*value)],
                    Step::Input(index) => self.inputs[*index].clone(),
                    Step::Load(slot) => {
                        self.check_initialized(&locals[*slot]);
                        locals[*slot]
                            .lanes
                            .iter()
                            .map(|&var| self.builder.use_var(var))
                            .collect()
                    }
                    Step::Store(slot, value) => {
                        let local = &locals[*slot];
                        for (&var, &reg) in local.lanes.iter().zip(&temps[*value]) {
                            self.builder.def_var(var, reg);
                        }
                        if let Some(var) = local.initialized {
                            let initialized = self.constant(1);
                            self.builder.def_var(var, initialized);
                        }
                        Vec::new()
                    }
                    Step::Update(slot, field, value) => {
                        let local = &locals[*slot];
                        self.check_initialized(local);
                        self.builder.def_var(local.lanes[0], zero);
                        self.builder
                            .def_var(local.lanes[field + 1], temps[*value][0]);
                        Vec::new()
                    }
                    Step::Field(record, field) => vec![temps[*record][field + 1]],
                    Step::Binary(kind, left, right) => {
                        vec![self.binary(*kind, temps[*left][0], temps[*right][0])]
                    }
                    Step::UnsignedBinary(kind, left, right) => {
                        vec![self.unsigned_binary(*kind, temps[*left][0], temps[*right][0])]
                    }
                    Step::Call(index, args) => {
                        let args = args.iter().map(|&arg| temps[arg].clone()).collect();
                        self.function(self.plan.function(*index), args)
                    }
                };
                temps.push(value);
            }
            match block.exit {
                Exit::Return(value) => {
                    let args: Vec<_> = temps[value]
                        .iter()
                        .copied()
                        .map(ir::BlockArg::Value)
                        .collect();
                    self.builder.ins().jump(done, &args);
                }
                Exit::Jump(target) => {
                    self.builder.ins().jump(blocks[target], &[]);
                }
                Exit::Branch(condition, yes, no) => {
                    self.builder
                        .ins()
                        .brif(temps[condition][0], blocks[yes], &[], blocks[no], &[]);
                }
            }
        }
        self.builder.switch_to_block(done);
        self.builder.block_params(done).to_vec()
    }

    fn check_initialized(&mut self, local: &Local) {
        if let Some(var) = local.initialized {
            let initialized = self.builder.use_var(var);
            let missing = self.builder.ins().icmp_imm_s(IntCC::Equal, initialized, 0);
            self.decline_if(missing);
        }
    }

    fn binary(&mut self, kind: Binary, a: Register, b: Register) -> Register {
        use Binary::*;
        let (value, overflow) = match kind {
            Add => self.builder.ins().sadd_overflow(a, b),
            Sub => self.builder.ins().ssub_overflow(a, b),
            Mul => self.builder.ins().smul_overflow(a, b),
            Div | Rem => {
                let zero = self.builder.ins().icmp_imm_s(IntCC::Equal, b, 0);
                let neg_one = self.builder.ins().icmp_imm_s(IntCC::Equal, b, -1);
                let min = self.builder.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                let overflow = self.builder.ins().band(neg_one, min);
                let invalid = self.builder.ins().bor(zero, overflow);
                self.decline_if(invalid);
                // Use trap-free operations after an explicit guard. Even a
                // remainder by -1 declines at signed minimum, like checked_rem.
                return if matches!(kind, Div) {
                    self.builder.ins().sdiv(a, b)
                } else {
                    self.builder.ins().srem(a, b)
                };
            }
            Eq | Lt | Le => {
                let condition = match kind {
                    Eq => IntCC::Equal,
                    Lt => IntCC::SignedLessThan,
                    _ => IntCC::SignedLessThanOrEqual,
                };
                let boolean = self.builder.ins().icmp(condition, a, b);
                return self.builder.ins().uextend(types::I64, boolean);
            }
            _ => unreachable!("lowered signed operation"),
        };
        self.decline_if(overflow);
        value
    }

    fn unsigned_binary(&mut self, kind: Binary, a: Register, b: Register) -> Register {
        use Binary::*;
        // The shared lane ABI carries bits in I64 registers. Unsigned
        // arithmetic must use carry/borrow/full-product overflow, not the
        // signed flag: crossing bit 63 is a valid u64 result. A guard failure
        // replays portable execution for its exact trap and resource order.
        let (value, overflow) = match kind {
            Add => self.builder.ins().uadd_overflow(a, b),
            Sub => self.builder.ins().usub_overflow(a, b),
            Mul => self.builder.ins().umul_overflow(a, b),
            Eq | Lt | Le => {
                let condition = match kind {
                    Eq => IntCC::Equal,
                    Lt => IntCC::UnsignedLessThan,
                    _ => IntCC::UnsignedLessThanOrEqual,
                };
                let boolean = self.builder.ins().icmp(condition, a, b);
                return self.builder.ins().uextend(types::I64, boolean);
            }
            _ => unreachable!("lowered unsigned operation"),
        };
        self.decline_if(overflow);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_ir::Block as NativeBlock;
    use super::*;
    use crate::structured::{BodyChecker, Limits, Source};
    use std::collections::BTreeSet;

    fn cost_block(fuel: u64, cells: usize, exit: Exit) -> NativeBlock {
        NativeBlock {
            fuel,
            cells,
            steps: Vec::new(),
            exit,
        }
    }

    // These graphs exercise reservation arithmetic, not machine-code emission.
    fn cost_plan(functions: Vec<Vec<NativeBlock>>) -> Plan {
        let module = Module::compile_compact("v2 (pub run () i64 = 0)").unwrap();
        let mut plan = Plan::lower(&module, module.exports["run"]).unwrap();
        plan.entry = 0;
        plan.functions = functions
            .into_iter()
            .map(|blocks| {
                Some(Function {
                    slots: Vec::new(),
                    params: 0,
                    result: Shape::I64,
                    blocks,
                })
            })
            .collect();
        plan
    }

    fn diamond() -> Vec<NativeBlock> {
        vec![
            cost_block(1, 2, Exit::Branch(0, 1, 2)),
            cost_block(3, 4, Exit::Jump(3)),
            cost_block(5, 1, Exit::Jump(3)),
            cost_block(7, 9, Exit::Return(0)),
        ]
    }

    #[test]
    fn acyclic_bound_joins_paths_and_maximizes_each_resource() {
        // The highest-fuel path and highest-cell path differ. The shared tail
        // and entry are each charged once per path, not once per predecessor.
        assert_eq!(acyclic_bound(&cost_plan(vec![diamond()])), Some((13, 15)));
    }

    #[test]
    fn acyclic_bound_counts_repeated_calls_but_ignores_unreachable_cycles() {
        let mut entry = cost_block(2, 3, Exit::Return(0));
        entry.steps = vec![Step::Call(1, vec![]), Step::Call(1, vec![])];
        let mut unreachable = cost_block(99, 99, Exit::Jump(1));
        unreachable.steps = vec![Step::Call(2, vec![])];
        let plan = cost_plan(vec![
            vec![entry, unreachable],
            diamond(),
            vec![cost_block(1, 1, Exit::Jump(0))],
        ]);
        assert_eq!(acyclic_bound(&plan), Some((28, 33)));
    }

    #[test]
    fn acyclic_bound_rejects_reachable_block_and_call_cycles() {
        let conditional_loop = cost_plan(vec![vec![
            cost_block(1, 1, Exit::Branch(0, 1, 2)),
            cost_block(1, 1, Exit::Return(0)),
            cost_block(1, 1, Exit::Jump(0)),
        ]]);
        assert_eq!(acyclic_bound(&conditional_loop), None);

        let mut caller = cost_block(1, 1, Exit::Return(0));
        caller.steps = vec![Step::Call(1, vec![])];
        let loop_callee = cost_plan(vec![vec![caller], vec![cost_block(1, 1, Exit::Jump(0))]]);
        assert_eq!(acyclic_bound(&loop_callee), None);

        let mut recursive = cost_block(1, 1, Exit::Return(0));
        recursive.steps = vec![Step::Call(0, vec![])];
        assert_eq!(acyclic_bound(&cost_plan(vec![vec![recursive]])), None);
    }

    #[test]
    fn acyclic_bound_rejects_overflow_in_paths_and_repeated_calls() {
        for (fuel, cells) in [(u64::MAX, 0), (0, usize::MAX)] {
            let path = cost_plan(vec![vec![
                cost_block(fuel, cells, Exit::Jump(1)),
                cost_block(1, 1, Exit::Return(0)),
            ]]);
            assert_eq!(acyclic_bound(&path), None);

            let mut caller = cost_block(0, 0, Exit::Return(0));
            caller.steps = vec![Step::Call(1, vec![]), Step::Call(1, vec![])];
            let calls = cost_plan(vec![
                vec![caller],
                vec![cost_block(fuel, cells, Exit::Return(0))],
            ]);
            assert_eq!(acyclic_bound(&calls), None);
        }
    }

    fn region_function(blocks: Vec<NativeBlock>) -> Function {
        Function {
            slots: Vec::new(),
            params: 0,
            result: Shape::I64,
            blocks,
        }
    }

    #[test]
    fn linear_regions_preserve_branch_costs_and_share_unconditional_suffixes() {
        let function = region_function(diamond());
        assert_eq!(
            linear_region_meters(&function),
            Some(vec![(1, 2), (10, 13), (12, 10), (0, 0)])
        );
        let function = region_function(vec![
            cost_block(2, 3, Exit::Jump(1)),
            cost_block(4, 0, Exit::Branch(0, 2, 3)),
            cost_block(5, 0, Exit::Jump(1)),
            cost_block(2, 0, Exit::Return(0)),
        ]);
        assert_eq!(
            linear_region_meters(&function),
            Some(vec![(6, 3), (0, 0), (9, 0), (2, 0)])
        );
    }

    #[test]
    fn linear_regions_keep_metered_jump_cycles_and_reject_invalid_or_overflowing_graphs() {
        let function = region_function(vec![
            cost_block(1, 2, Exit::Jump(1)),
            cost_block(3, 4, Exit::Jump(2)),
            cost_block(5, 6, Exit::Jump(1)),
            cost_block(7, 8, Exit::Jump(3)), // Unreachable self-cycle.
        ]);
        assert_eq!(
            linear_region_meters(&function),
            Some(vec![(1, 2), (8, 10), (0, 0), (7, 8)])
        );
        for blocks in [
            Vec::new(),
            vec![cost_block(1, 1, Exit::Jump(1))],
            vec![cost_block(1, 1, Exit::Branch(0, 0, 1))],
            vec![
                cost_block(u64::MAX, 0, Exit::Jump(1)),
                cost_block(1, 0, Exit::Return(0)),
            ],
            vec![
                cost_block(0, usize::MAX, Exit::Jump(1)),
                cost_block(0, 1, Exit::Return(0)),
            ],
        ] {
            assert!(linear_region_meters(&region_function(blocks)).is_none());
        }
    }

    #[test]
    fn linear_region_reservations_cover_every_prefix_and_charge_exact_complete_paths() {
        // This path oracle independently accumulates the old per-block and new
        // emitted charges. Every prefix must be funded; every completed path
        // must have identical totals, including repeated loop visits.
        fn walk(
            function: &Function,
            meters: &[ResourceBound],
            index: usize,
            before: ResourceBound,
            reserved: ResourceBound,
            remaining: usize,
            completions: &mut usize,
        ) {
            let block = &function.blocks[index];
            let before = add_bound(before, (block.fuel, block.cells)).unwrap();
            let reserved = add_bound(reserved, meters[index]).unwrap();
            assert!(reserved.0 >= before.0 && reserved.1 >= before.1);
            if let Exit::Return(_) = block.exit {
                assert_eq!(reserved, before);
                *completions += 1;
                return;
            }
            if remaining == 0 {
                return;
            }
            let mut next = |index| {
                walk(
                    function,
                    meters,
                    index,
                    before,
                    reserved,
                    remaining - 1,
                    completions,
                );
            };
            match block.exit {
                Exit::Jump(index) => next(index),
                Exit::Branch(_, yes, no) => {
                    next(yes);
                    next(no);
                }
                Exit::Return(_) => unreachable!(),
            }
        }
        let graphs = [
            diamond(),
            vec![
                cost_block(1, 2, Exit::Jump(1)),
                cost_block(3, 4, Exit::Branch(0, 2, 7)),
                cost_block(5, 6, Exit::Jump(3)),
                cost_block(7, 8, Exit::Branch(0, 4, 5)),
                cost_block(11, 12, Exit::Jump(3)),
                cost_block(13, 14, Exit::Branch(0, 6, 7)),
                cost_block(17, 18, Exit::Jump(1)),
                cost_block(19, 20, Exit::Return(0)),
            ],
            // The cycle has two entries and no single dominating loop header.
            vec![
                cost_block(1, 2, Exit::Branch(0, 1, 2)),
                cost_block(3, 4, Exit::Jump(3)),
                cost_block(5, 6, Exit::Jump(3)),
                cost_block(7, 8, Exit::Branch(0, 1, 4)),
                cost_block(11, 12, Exit::Branch(0, 2, 5)),
                cost_block(13, 14, Exit::Return(0)),
            ],
        ];
        for blocks in graphs {
            let function = region_function(blocks);
            let meters = linear_region_meters(&function).unwrap();
            let mut completions = 0;
            walk(&function, &meters, 0, (0, 0), (0, 0), 16, &mut completions);
            assert!(completions > 0);
        }
    }

    #[test]
    fn acyclic_bound_preserves_shared_plan_and_straight_line_admission() {
        let source = "v2 (fn add (x:i64) i64 = (+ x 1))
                      (pub run (x:i64) i64 = (call add (call add x)))";
        let module = Module::compile_compact(source).unwrap();
        let plan = Plan::lower(&module, module.exports["run"]).unwrap();
        assert!(plan.boundary.fixed.is_some());
        assert_eq!(acyclic_bound(&plan), plan.boundary.fixed);

        let module =
            Module::compile_compact(include_str!("../../examples/fixtures/host_decision.flux"))
                .unwrap();
        let plan = Plan::lower(&module, module.exports["decide"]).unwrap();
        assert_eq!(plan.boundary.fixed, None);
        assert!(acyclic_bound(&plan).is_some());
        // Merely analyzing a shared Plan must not change the AOT admission
        // contract. Only Native::compile_jit installs the stronger private bound.
        assert_eq!(plan.boundary.fixed, None);
    }

    #[test]
    fn initialization_proof_keeps_partial_join_and_loop_entry_guards() {
        let cases = [
            (
                "v2 (pub run (x:i64) i64 (y:i64) (= y (+ x 1)) (ret y))",
                vec![false, false],
            ),
            (
                "v2 (pub run (x:i64) i64 (y:i64) (if (< x 0) ((= y 1)) ((= y 2))) (ret y))",
                vec![false, false],
            ),
            (
                "v2 (pub run (x:i64) i64 (y:i64) (if (< x 0) ((= y 1)) ()) (ret y))",
                vec![false, true],
            ),
            (
                "v2 (pub run (n:i64) i64 (i:i64 y:i64) (= i 0) (while (< i n) ((= y i) (= i (+ i 1)))) (ret y))",
                vec![false, false, true],
            ),
            (
                "v2 (pub run (n:i64) i64 (i:i64 y:i64) (= i 0) (= y 0) (while (< i n) ((= y (+ y i)) (= i (+ i 1)))) (ret y))",
                vec![false, false, false],
            ),
            (
                "v2 (pub run (n:i64) i64 (i:i64 y:i64 z:i64) (= i 0) (= z 0) (while (< i n) ((= z y) (= y 1) (= i (+ i 1)))) (ret z))",
                vec![false, false, true, false],
            ),
            (
                "v2 (rec R x:i64) (pub run (n:i64) R (s:R) (= s.x n) (ret s))",
                vec![false, true],
            ),
        ];
        for (source, expected) in cases {
            let module = Module::compile_compact(source).unwrap();
            let plan = Plan::lower(&module, module.exports["run"]).unwrap();
            assert_eq!(
                initialization_guards(plan.function(plan.entry)),
                expected,
                "{source}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_published_code_is_rx_and_has_registered_systemv_cfi() {
        use std::ffi::c_void;
        #[repr(C)]
        struct Bases {
            text: *mut c_void,
            data: *mut c_void,
            function: *mut c_void,
        }
        #[link(name = "gcc_s")]
        unsafe extern "C" {
            fn _Unwind_Find_FDE(pc: *const c_void, bases: *mut Bases) -> *const c_void;
        }
        let module = Module::compile_compact("v2 (pub run (x:i64) i64 () (ret (+ x 9)))").unwrap();
        let Native::Scalar(jit) = Native::compile(&module, module.exports["run"]).unwrap() else {
            panic!("fixed signed export must use the scalar owner");
        };
        let entry = jit.page.entry();
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        let permissions = maps
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let (start, end) = fields.next()?.split_once('-')?;
                let start = usize::from_str_radix(start, 16).ok()?;
                let end = usize::from_str_radix(end, 16).ok()?;
                (start <= entry as usize && (entry as usize) < end).then(|| fields.next().unwrap())
            })
            .unwrap();
        assert!(permissions.starts_with("r-x"), "{permissions}");
        let mut bases = Bases {
            text: std::ptr::null_mut(),
            data: std::ptr::null_mut(),
            function: std::ptr::null_mut(),
        };
        // SAFETY: libgcc inspects its own registered frame tables. The page and
        // output structure are alive for this lookup; no generated code is run.
        let fde = unsafe { _Unwind_Find_FDE(entry, &mut bases) };
        assert!(!fde.is_null(), "generated PC must have a registered FDE");
        assert_eq!(bases.function.cast_const(), entry);
        let args = [Value::I64(7)];
        assert_eq!(
            jit.run(&Invocation {
                export: "run",
                inputs: &[],
                arguments: &args,
                limits: Limits::default()
            }),
            Some(Value::I64(16))
        );
    }

    #[test]
    fn explicit_owners_preserve_abi_lane_limits_and_record_identity() {
        let module = Module::compile_compact(
            "v2
             (rec R a:i64 b:i64 c:i64 d:i64)
             (pub scalar (x:i64) i64 = (+ x 9))
             (pub predicate (x:i64 y:i64) bool = (< x y))
             (pub choose (flag:bool x:i64 y:i64) i64 ()
               (if flag ((ret x)) ((ret y))))
             (pub wide (a:i64 b:i64 c:i64 d:i64 e:i64) i64 = (+ (+ (+ (+ a b) c) d) e))
             (pub record (s:R) R = s)
             (pub dynamic (n:i64) i64 (sum:i64) (= sum 0)
               (while (< 0 n) ((= sum (+ sum n)) (= n (- n 1)))) (ret sum))
             (pub scan (xs:[f32]) u64 = (minidx xs))",
        )
        .unwrap();
        let record: Arc<[Value]> =
            [Value::I64(2), Value::I64(3), Value::I64(5), Value::I64(7)].into();
        let cases = [
            ("scalar", "scalar", vec![Value::I64(7)], Value::I64(16)),
            (
                "predicate",
                "tagged_scalar",
                vec![Value::I64(-3), Value::I64(8)],
                Value::Bool(true),
            ),
            (
                "choose",
                "tagged_scalar",
                vec![Value::Bool(false), Value::I64(-3), Value::I64(8)],
                Value::I64(8),
            ),
            (
                "wide",
                "wide",
                vec![
                    Value::I64(1),
                    Value::I64(-2),
                    Value::I64(3),
                    Value::I64(-4),
                    Value::I64(5),
                ],
                Value::I64(3),
            ),
            (
                "record",
                "wide",
                vec![Value::Record(record.clone())],
                Value::Record(record.clone()),
            ),
            ("dynamic", "small", vec![Value::I64(8)], Value::I64(36)),
            (
                "scan",
                "scan",
                vec![Value::List(
                    vec![Value::F32(0x3f80_0000), Value::F32(0xbf80_0000)].into(),
                )],
                Value::U64(1),
            ),
        ];
        for (export, owner, args, expected) in cases {
            let native = Native::compile(&module, module.exports[export]).unwrap();
            assert_eq!(
                match &native {
                    Native::Scalar(_) => "scalar",
                    Native::TaggedScalar(_) => "tagged_scalar",
                    Native::Small(_) => "small",
                    Native::Wide(_) => "wide",
                    Native::Scan(_) => "scan",
                },
                owner,
                "{export}",
            );
            assert_eq!(
                native.route(),
                if export == "scan" {
                    super::super::ExecutionRoute::PrecompiledScan
                } else {
                    super::super::ExecutionRoute::TypedJit
                },
            );
            let call = Invocation {
                export,
                inputs: &[],
                arguments: &args,
                limits: Limits::default(),
            };
            let value = native.run(&call).unwrap();
            assert_eq!(value, expected, "{export}");
            if let Value::Record(fields) = value {
                assert!(Arc::ptr_eq(&fields, &record));
            }
            for limits in [
                Limits {
                    instructions: 0,
                    ..call.limits
                },
                Limits {
                    cells: 0,
                    ..call.limits
                },
                Limits {
                    frames: 0,
                    ..call.limits
                },
            ] {
                assert_eq!(native.run(&Invocation { limits, ..call }), None, "{export}");
            }
            let mut malformed = args.clone();
            malformed[0] = Value::Unit;
            assert_eq!(
                native.run(&Invocation {
                    arguments: &malformed,
                    ..call
                }),
                None,
                "{export}",
            );
            assert_eq!(native.run(&call), Some(expected), "{export}");
        }
    }

    #[test]
    fn reused_caller_uses_new_native_callee() {
        let text = "v2\n(fn add (x:i64) i64 () (ret (+ x 1)))\n(pub run (x:i64) i64 (y:i64) (= y (call add x)) (ret (call add y)))";
        let original = Module::compile_compact(text).unwrap();
        let index = original.exports["run"];
        let args = [Value::I64(10)];
        let call = || Invocation {
            export: "run",
            inputs: &[],
            arguments: &args,
            limits: Limits::default(),
        };
        assert!(original.prepare_native("run"));
        let old = original.native[index].get().unwrap().as_ref().unwrap();
        assert_eq!(old.run(&call()), Some(Value::I64(12)));

        let source = Source::parse_compact(&text.replace("(+ x 1)", "(+ x 2)")).unwrap();
        let checker = BodyChecker::new(&source).unwrap();
        let next = original
            .replace_bodies(&source, &checker, &BTreeSet::from([0]))
            .unwrap();
        assert!(Arc::ptr_eq(&next.code[index], &original.code[index]));
        assert!(next.native[index].get().is_none());
        assert_eq!(next.run(call()).unwrap(), Value::I64(14));
        let new = next.native[index].get().unwrap().as_ref().unwrap();
        assert_eq!(new.run(&call()), Some(Value::I64(14)));
        assert_eq!(old.run(&call()), Some(Value::I64(12)));
    }

    #[test]
    fn runtime_meter_admits_counter_and_declines_extreme_errors() {
        let module = Module::compile_legacy_json(include_str!(
            "../../tests/fixtures/structured_counter.fluxm"
        ))
        .unwrap();
        let native = Native::compile(&module, module.exports["tick"]).unwrap();
        let inputs = [Value::I64(3)];
        let args = [Value::Record([Value::I64(10)].into()), Value::I64(19)];
        let limits = Limits::default();
        assert_eq!(
            native.run(&Invocation {
                export: "tick",
                inputs: &inputs,
                arguments: &args,
                limits
            }),
            Some(Value::Record([Value::I64(202)].into()))
        );
        assert_eq!(
            native.run(&Invocation {
                export: "tick",
                inputs: &inputs,
                arguments: &args,
                limits: Limits {
                    instructions: 50,
                    ..limits
                }
            }),
            None
        );
        let bad = [Value::Record([Value::I64(i64::MAX)].into()), Value::I64(19)];
        assert_eq!(
            native.run(&Invocation {
                export: "tick",
                inputs: &inputs,
                arguments: &bad,
                limits
            }),
            None
        );
    }
}
