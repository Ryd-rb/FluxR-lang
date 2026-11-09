use super::{COUNTER, DECISION, SCAN, VOLUME, corpus_ref, scan_values};
use fluxr::Error;
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};

const FUEL: &str = "module instruction budget exhausted";
const CELLS: &str = "module cell budget exhausted";
const FRAMES: &str = "module frame limit exceeded";
const MATH: &str = "invalid or overflowing integer arithmetic";

#[cfg(any(test, target_arch = "wasm32"))]
struct Budget(Limits);

#[cfg(any(test, target_arch = "wasm32"))]
impl Budget {
    fn fuel(&mut self, amount: u64) -> Result<(), &'static str> {
        self.0.instructions = self.0.instructions.checked_sub(amount).ok_or(FUEL)?;
        Ok(())
    }

    fn cells(&mut self, amount: usize) -> Result<(), &'static str> {
        self.0.cells = self.0.cells.checked_sub(amount).ok_or(CELLS)?;
        Ok(())
    }

    fn enter(&mut self, depth: usize, slots: usize) -> Result<(), &'static str> {
        if depth > self.0.frames {
            return Err(FRAMES);
        }
        self.cells(slots + 1)
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
#[path = "binary32_boundary_checks.rs"]
mod binary32_boundary_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use binary32_boundary_checks::check_binary32_boundary;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "compound_effect_checks.rs"]
mod compound_effect_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use compound_effect_checks::check_compound_effects;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "control_flow_checks.rs"]
mod control_flow_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use control_flow_checks::check_control_flow;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "list_boundary_checks.rs"]
mod list_boundary_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use list_boundary_checks::check_list_boundary;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "list_effect_checks.rs"]
mod list_effect_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use list_effect_checks::check_list_effects;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "loop_exits_checks.rs"]
mod loop_exits_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use loop_exits_checks::check_loop_exits;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "method_call_order_checks.rs"]
mod method_call_order_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use method_call_order_checks::{check_method_call_order, check_method_edit_isolation};
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "nested_loop_calls_checks.rs"]
mod nested_loop_calls_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use nested_loop_calls_checks::check_nested_loop_calls;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "recursive_effect_checks.rs"]
mod recursive_effect_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use recursive_effect_checks::check_recursive_effects;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "shared_host_admission_checks.rs"]
mod shared_host_admission_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use shared_host_admission_checks::check_shared_host_admission;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "structured_boundary_checks.rs"]
mod structured_boundary_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use structured_boundary_checks::check_structured_boundary;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "typed_effect_checks.rs"]
mod typed_effect_checks;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "typed_effect_spec.rs"]
mod typed_effect_spec;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use typed_effect_checks::check_typed_effects;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "universal_tape_checks.rs"]
mod universal_tape_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use universal_tape_checks::check_universal_tape;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "variant_payload_checks.rs"]
mod variant_payload_checks;
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) use variant_payload_checks::check_variant_payload;

struct Replay<'a> {
    export: &'a str,
    inputs: &'a [Value],
    direct: Module,
    decoded: Module,
    injected: ModuleSlot,
    workspace: Workspace,
    count: usize,
}

impl<'a> Replay<'a> {
    fn new(module: Module, export: &'a str, inputs: &'a [Value], grants: &[Binding]) -> Self {
        let bytes = module.encode();
        let decoded = Module::decode(&bytes).unwrap();
        assert_eq!(decoded.encode(), bytes);
        module.prepare_native(export);
        let mut injected = ModuleSlot::new(decoded.clone(), grants).unwrap();
        let candidate = Module::decode(&bytes).unwrap();
        candidate.prepare_native(export);
        assert_eq!(injected.inject(0, candidate, None).unwrap(), 1);
        Self {
            export,
            inputs,
            direct: module,
            decoded,
            injected,
            workspace: Workspace::default(),
            count: 0,
        }
    }

    fn check(&mut self, arguments: &[Value], limits: Limits, expected: Result<Value, &str>) {
        self.check_call(self.inputs, arguments, limits, expected);
    }

    fn check_call(
        &mut self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        expected: Result<Value, &str>,
    ) {
        self.check_routes(inputs, arguments, limits, expected, |_| {});
    }

    fn check_routes(
        &mut self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        expected: Result<Value, &str>,
        inspect: impl Fn(&Value),
    ) {
        self.check_routes_labeled(
            inputs,
            arguments,
            limits,
            expected,
            format_args!("inputs {inputs:?}, arguments {arguments:?}"),
            inspect,
        );
    }

    fn check_routes_labeled(
        &mut self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        expected: Result<Value, &str>,
        label: impl std::fmt::Display,
        inspect: impl Fn(&Value),
    ) {
        let call = || Invocation {
            export: self.export,
            inputs,
            arguments,
            limits,
        };
        let workspace = &mut self.workspace;
        for (route, result) in [
            ("direct", self.direct.run_with(call(), workspace)),
            ("portable", self.direct.run_portable_with(call(), workspace)),
            ("decoded", self.decoded.run_portable_with(call(), workspace)),
            ("injected", self.injected.run_with(call(), workspace)),
        ] {
            let result: Result<Value, String> = result.map_err(|e: Error| e.to_string());
            assert_eq!(
                result.as_ref().map_err(String::as_str),
                expected.as_ref().map_err(|&err| err),
                "{}.{} {limits:?}, {label}",
                self.export,
                route
            );
            if let Ok(value) = &result {
                inspect(value);
            }
        }
        self.count += 1;
    }
}

pub(super) fn check() -> usize {
    let limits = Limits::default();
    let fuel = Limits {
        instructions: 0,
        ..limits
    };
    let cells = Limits { cells: 0, ..limits };
    let frames = Limits {
        frames: 0,
        ..limits
    };

    let mut volume = Replay::new(Module::compile_compact(VOLUME).unwrap(), "volume", &[], &[]);
    let zero = [0; 11].map(Value::I64);
    volume.check(&zero, limits, Ok(Value::I64(0)));
    let mut overflow = zero.clone();
    overflow[0] = Value::I64(i64::MIN);
    overflow[1] = Value::I64(i64::MAX);
    volume.check(&overflow, limits, Err(MATH));
    volume.check(&overflow, fuel, Err(FUEL));
    overflow[0] = Value::I64(0);
    overflow[8] = Value::I64(2);
    volume.check(&overflow, limits, Err(MATH));
    volume.check(
        &zero[..10],
        limits,
        Err("host argument or input count differs from interface"),
    );

    let mut counter = Replay::new(
        Module::compile_legacy_json(COUNTER).unwrap(),
        "tick",
        &[Value::I64(3)],
        &[Binding {
            name: "bonus".into(),
            ty: Type::I64,
        }],
    );
    let record = |total| Value::Record([Value::I64(total)].into());
    let args = [record(10), Value::I64(6)];
    counter.check(&args, limits, Ok(record(37)));
    counter.check(&[record(10), Value::I64(-1)], limits, Ok(record(10)));
    counter.check(&[record(i64::MAX), Value::I64(1)], limits, Err(MATH));
    counter.check(&args, cells, Err(CELLS));
    counter.check(
        &args,
        Limits {
            frames: 2,
            ..limits
        },
        Err(FRAMES),
    );

    let mut scan = Replay::new(Module::compile_compact(SCAN).unwrap(), "pick", &[], &[]);
    let nan = f32::from_bits(0x7fc0_0031);
    for (values, index) in [
        (&[][..], 0),
        (&[2., 1., 1.], 1),
        (&[nan, -1., f32::NEG_INFINITY], 0),
        (&[1., nan, -1., -1.], 2),
        (&[0., -0.], 0),
        (&[1., f32::from_bits(0x7f80_0001), 0.], 2),
    ] {
        assert_eq!(corpus_ref::pick(values.iter().copied()), index);
        scan.check(&[scan_values(values)], limits, Ok(Value::U64(index)));
    }
    let list = [scan_values(&[2., 1., 1.])];
    let exact = Limits {
        instructions: 6,
        cells: 7,
        frames: 1,
    };
    scan.check(&list, exact, Ok(Value::U64(1)));
    scan.check(
        &list,
        Limits {
            instructions: 5,
            ..exact
        },
        Err(FUEL),
    );
    scan.check(&list, Limits { cells: 6, ..exact }, Err(CELLS));
    scan.check(&list, frames, Err(FRAMES));
    let malformed = [Value::List(vec![Value::F32(0), Value::I64(1)].into())];
    scan.check(&malformed, limits, Err("host value differs from F32"));
    scan.check(&malformed, Limits { cells: 2, ..limits }, Err(CELLS));

    let mut decision = Replay::new(
        Module::compile_compact(DECISION).unwrap(),
        "decide",
        &[],
        &[],
    );
    for (i, answer) in [21, 19, 40, 10, 23, 17, 80, 5].into_iter().enumerate() {
        let sign = |mask| if i & mask == 0 { -1 } else { 0 };
        let input = [sign(4), sign(2), sign(1), 20];
        assert_eq!(corpus_ref::decide(input), Some(answer));
        decision.check(&input.map(Value::I64), limits, Ok(Value::I64(answer)));
    }
    let extreme = [-1, -1, -1, i64::MAX].map(Value::I64);
    decision.check(&extreme, limits, Err(MATH));
    decision.check(&extreme, fuel, Err(FUEL));
    let extreme = [-1, -1, 0, i64::MIN].map(Value::I64);
    decision.check(&extreme, limits, Err(MATH));
    let extreme = [0, 0, -1, i64::MAX].map(Value::I64);
    decision.check(&extreme, limits, Err(MATH));
    let extreme = [0, 0, 0, i64::MIN].map(Value::I64);
    decision.check(&extreme, limits, Ok(Value::I64(i64::MIN / 4)));
    volume.count + counter.count + scan.count + decision.count
}
