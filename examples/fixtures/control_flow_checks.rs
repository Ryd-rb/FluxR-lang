//! An independent source ledger for signed branches and backedges.
use super::{Budget, CELLS, FRAMES, FUEL, MATH, Replay};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Value, Workspace};
use std::sync::Arc;

mod aot {
    include!("control_flow_aot.rs");
}

const SOURCE: &str = include_str!("control_flow.fluxr");
const EXPORT: &str = "walk";
const PREPARED: bool = cfg!(all(
    feature = "typed-jit",
    target_os = "windows",
    target_arch = "x86_64"
));
const EXACT: Limits = limits(60, 18, 2);
const REUSE: Limits = limits(6, 13, 1);

const fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn record(total: i64, marker: i64) -> Value {
    Value::Record([Value::I64(total), Value::I64(marker)].into())
}

fn args(total: i64, n: i64, step: i64) -> [Value; 3] {
    [record(total, -11), Value::I64(n), Value::I64(step)]
}

fn integer(budget: &mut Budget, value: &Value) -> Result<i64, &'static str> {
    budget.cells(1)?;
    match value {
        Value::I64(n) => Ok(*n),
        _ => Err("host value differs from I64"),
    }
}

fn oracle(
    inputs: &[Value],
    arguments: &[Value],
    limits: Limits,
    subtract: bool,
) -> Result<Value, &'static str> {
    let ([], [state, n, step]) = (inputs, arguments) else {
        return Err("host argument or input count differs from interface");
    };
    let mut budget = Budget(limits);
    budget.cells(1)?;
    let Value::Record(fields) = state else {
        return Err("host value differs from Record(\"State\")");
    };
    let [total, marker] = &**fields else {
        return Err("host value differs from Record(\"State\")");
    };
    let mut total = integer(&mut budget, total)?;
    let marker = integer(&mut budget, marker)?;
    let mut n = integer(&mut budget, n)?;
    let step = integer(&mut budget, step)?;
    budget.enter(1, 4)?;
    budget.fuel(4)?; // Signed early return condition and branch.
    let result = if n <= 0 {
        state.clone()
    } else {
        budget.fuel(3)?; // Read the total field and initialize the accumulator.
        loop {
            budget.fuel(4)?; // The loop test also runs on exit.
            if n <= 1 {
                break;
            }
            budget.fuel(3)?;
            total = if subtract {
                total.checked_sub(step)
            } else {
                total.checked_add(step)
            }
            .ok_or(MATH)?;
            budget.fuel(1)?; // Store the checked result.
            budget.fuel(4)?; // Decrement and store. The loop guard makes it safe.
            n -= 1;
            budget.fuel(1)?; // Backedge.
        }
        budget.fuel(3)?; // Read quotient arguments and dispatch.
        budget.enter(2, 2)?;
        budget.fuel(3)?;
        total = total.checked_div(step).ok_or(MATH)?;
        budget.fuel(1)?; // Return from quotient.
        budget.fuel(1)?;
        budget.cells(2)?; // Detach the two host fields on the first write.
        record(total, marker)
    };
    budget.fuel(2)?; // Read and return the record.
    budget.cells(3)?; // Admit the record and both fields at the boundary.
    Ok(result)
}

fn call<'a>(inputs: &'a [Value], arguments: &'a [Value], limits: Limits) -> Invocation<'a> {
    Invocation {
        export: EXPORT,
        inputs,
        arguments,
        limits,
    }
}

fn check_ownership(arguments: &[Value], expected: &Value, actual: &Value) {
    let (Value::Record(host), Value::Record(want), Value::Record(got)) =
        (&arguments[0], expected, actual)
    else {
        panic!("successful walk returns a record");
    };
    assert_eq!(Arc::ptr_eq(host, got), Arc::ptr_eq(host, want));
}

fn check_probe(module: Module, good: &[Value], reuse: &[Value]) {
    let original = module.encode();
    let replacement = Module::compile_compact(&SOURCE.replace("(+ total step)", "(- total step)"))
        .unwrap()
        .encode();
    assert_ne!(original, replacement);
    let mut slot = ModuleSlot::new(module, &[]).unwrap();
    let mut workspace = Workspace::default();
    let old = oracle(&[], good, EXACT, false).unwrap();
    let new = oracle(&[], good, EXACT, true).unwrap();
    assert_eq!(old, record(6, -11));
    assert_eq!(new, record(0, -11));
    for (arguments, budget, trap) in [
        (good, limits(59, 18, 2), FUEL),
        (good, limits(60, 17, 2), CELLS),
        (good, limits(60, 18, 1), FRAMES),
        (&args(i64::MIN, 2, 1), EXACT, MATH),
        (&args(i64::MIN, 3, 0), EXACT, MATH),
    ] {
        assert_eq!(oracle(&[], arguments, budget, true), Err(trap));
        let candidate = Module::decode(&replacement).unwrap();
        assert_eq!(candidate.prepare_native(EXPORT), PREPARED);
        assert_eq!(
            slot.inject(0, candidate, Some(call(&[], arguments, budget)))
                .unwrap_err()
                .to_string(),
            trap
        );
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        let actual = slot
            .run_with(call(&[], good, EXACT), &mut workspace)
            .unwrap();
        assert_eq!(actual, old);
        check_ownership(good, &old, &actual);
        let actual = slot
            .run_with(call(&[], reuse, REUSE), &mut workspace)
            .unwrap();
        assert_eq!(actual, reuse[0]);
        check_ownership(reuse, &reuse[0], &actual);
    }
    let candidate = Module::decode(&replacement).unwrap();
    assert_eq!(candidate.prepare_native(EXPORT), PREPARED);
    assert_eq!(
        slot.inject(0, candidate, Some(call(&[], good, EXACT)))
            .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), replacement);
    let actual = slot
        .run_with(call(&[], good, EXACT), &mut workspace)
        .unwrap();
    assert_eq!(actual, new);
    check_ownership(good, &new, &actual);
}

pub(crate) fn check_control_flow() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert_eq!(module.prepare_native(EXPORT), PREPARED);
    let aot = aot::prepare(&module).unwrap();
    let mut replay = Replay::new(module.clone(), EXPORT, &[], &[]);
    assert_eq!(replay.injected.module().prepare_native(EXPORT), PREPARED);
    let mut workspace = Workspace::default();
    let mut check = |inputs: &[Value], arguments: &[Value], budget| {
        let expected = oracle(inputs, arguments, budget, false);
        let inspect =
            |actual: &Value| check_ownership(arguments, expected.as_ref().unwrap(), actual);
        replay.check_routes(inputs, arguments, budget, expected.clone(), inspect);
        let actual = aot.run_with(call(inputs, arguments, budget), &mut workspace);
        assert_eq!(
            actual.as_ref().map_err(ToString::to_string),
            expected.as_ref().map_err(ToString::to_string),
            "walk.aot {budget:?}, inputs {inputs:?}, arguments {arguments:?}"
        );
        if let Ok(value) = &actual {
            inspect(value);
        }
    };
    for (total, n, step) in [
        (i64::MAX, 0, 0),
        (i64::MIN, i64::MIN, -1),
        (i64::MAX, 1, 2),
        (i64::MIN, 1, -1),
        (7, 1, 0),
        (i64::MAX, 2, 1),
        (0, 2, 0),
        (-4, 2, -2),
        (7, 4, 2),
        (i64::MAX - 2, 4, 1),
        (i64::MIN + 1, 4, -1),
    ] {
        let arguments = args(total, n, step);
        for cells in 0..=19 {
            for fuel in 0..=61 {
                for frames in 0..=2 {
                    check(&[], &arguments, limits(fuel, cells, frames));
                }
            }
        }
        assert_eq!(arguments, args(total, n, step));
    }
    let good = args(7, 4, 2);
    let reuse = args(i64::MAX, 0, 0);
    for (inputs, arguments) in [
        (&[Value::Unit][..], &good[..]),
        (&[][..], &good[..2]),
        (&[][..], &[Value::Bool(false), Value::Unit, Value::Unit]),
        (
            &[][..],
            &[
                Value::Record([Value::I64(7), Value::Unit].into()),
                Value::Unit,
                Value::Unit,
            ],
        ),
        (
            &[][..],
            &[record(7, -11), Value::I64(4), Value::Bool(false)],
        ),
    ] {
        for cells in 0..=5 {
            for fuel in [0, 60] {
                for frames in 0..=2 {
                    check(inputs, arguments, limits(fuel, cells, frames));
                }
            }
        }
    }
    let deep = args(7, 1025, 2);
    for budget in [
        limits(13_333, 18, 2),
        limits(13_332, 18, 2),
        limits(13_333, 17, 2),
        limits(13_333, 18, 1),
        limits(13_334, 19, 3),
        limits(u64::MAX, usize::MAX, usize::MAX),
    ] {
        check(&[], &deep, budget);
    }
    assert_eq!(deep, args(7, 1025, 2));
    let huge = args(7, i64::MAX, 2);
    check(&[], &huge, EXACT);
    check(&[], &huge, limits(0, 0, 0));
    check(&[], &good, EXACT);
    check(&[], &reuse, REUSE);
    check_probe(module.clone(), &good, &reuse);
    assert_eq!(good, args(7, 4, 2));
    assert_eq!(reuse, args(i64::MAX, 0, 0));
    replay.count
}
