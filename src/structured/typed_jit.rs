//! Native execution for the shared typed subset on Win64.
use super::compile::Operand;
use super::native_ir::{Boundary, Exit, Function, MAX_LANES, Plan, Shape, Step};
use super::vm::{min_f32_index, scan_effect};
use super::{Binary, Invocation, Module, Type, Value};
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
    Jit(Jit),
    Scan(Scan),
}

impl Native {
    pub(super) fn compile(module: &Module, index: usize) -> Option<Self> {
        if let Some(scan) = Scan::lower(module, index) {
            return Some(Self::Scan(scan));
        }
        Jit::compile(module, index).map(Self::Jit)
    }

    pub(super) fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        match self {
            Self::Jit(jit) => jit.run(invocation),
            Self::Scan(scan) => scan.run(invocation),
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
            || !values.iter().all(|value| matches!(value, Value::F32(_)))
        {
            return None;
        }
        Some(Value::U64(min_f32_index(values)))
    }
}

#[derive(Debug)]
pub(super) struct Jit {
    boundary: Boundary,
    page: RegisteredPage,
    small: bool,
}

impl Jit {
    fn compile(module: &Module, index: usize) -> Option<Self> {
        let plan = Plan::lower(module, index)?;
        let mut signature = ir::Signature::new(CallConv::WindowsFastcall);
        signature.params.extend([ir::AbiParam::new(types::I64); 4]);
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
        let UnwindInfo::WindowsX64(info) = code.create_unwind_info(target).ok()?? else {
            return None;
        };
        let mut unwind = vec![0; info.emit_size()];
        info.emit(&mut unwind);
        let page = RegisteredPage::publish(code.code_buffer(), &unwind).ok()?;
        let small = plan.boundary.fits_lanes(SMALL_LANES);
        Some(Self {
            boundary: plan.boundary,
            page,
            small,
        })
    }

    fn run(&self, invocation: &Invocation<'_>) -> Option<Value> {
        if self.small {
            self.run_with_lanes::<SMALL_LANES>(invocation)
        } else {
            self.run_with_lanes::<MAX_LANES>(invocation)
        }
    }

    fn run_with_lanes<const LANES: usize>(&self, invocation: &Invocation<'_>) -> Option<Value> {
        debug_assert!(self.boundary.fits_lanes(LANES));
        self.boundary
            .run::<LANES>(invocation, |input, output, fuel, cells| {
                type Entry = unsafe extern "system" fn(*const i64, *mut i64, u64, u64) -> u32;
                // SAFETY: The immutable RX owner outlives this call. The only memory
                // operations are the input loads and result stores emitted for the
                // same boundary that selected LANES.
                // Arithmetic guards prevent division faults, and the page supplies
                // registered Windows unwind metadata for stack walking.
                unsafe {
                    let entry: Entry = std::mem::transmute(self.page.entry());
                    entry(input.as_ptr(), output.as_mut_ptr(), fuel, cells)
                }
            })
    }
}

fn target() -> Option<&'static dyn TargetIsa> {
    static ISA: OnceLock<Option<Arc<dyn TargetIsa>>> = OnceLock::new();
    ISA.get_or_init(|| {
        let mut flags = settings::builder();
        flags.set("opt_level", "speed").ok()?;
        flags.set("enable_verifier", "true").ok()?;
        flags.set("enable_probestack", "true").ok()?;
        isa::lookup_by_name("x86_64-pc-windows-msvc")
            .ok()?
            .finish(settings::Flags::new(flags))
            .ok()
    })
    .as_deref()
}

fn emit(mut builder: FunctionBuilder<'_>, plan: &Plan, config: TargetFrontendConfig) {
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
    let results = emit.function(plan.function(plan.entry), args);
    for (lane, result) in results.into_iter().enumerate() {
        emit.builder.ins().store(
            ir::MemFlagsData::trusted(),
            result,
            abi[1],
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
        let locals: Vec<_> = function
            .slots
            .iter()
            .enumerate()
            .map(|(index, shape)| Local {
                lanes: (0..shape.lanes())
                    .map(|lane| self.variable(args.get(index).map_or(zero, |arg| arg[lane])))
                    .collect(),
                initialized: (index >= function.params).then(|| self.variable(zero)),
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
        for (block, &label) in function.blocks.iter().zip(&blocks) {
            self.builder.switch_to_block(label);
            self.meter(block.fuel, block.cells);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structured::{BodyChecker, Limits, Source};
    use std::collections::BTreeSet;

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
