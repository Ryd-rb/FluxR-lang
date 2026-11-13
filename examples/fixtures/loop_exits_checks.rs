//! Source ledgers for boolean short-circuiting and explicit signed loop exits.
use super::{Budget, MATH, Replay};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Value, Workspace};
use std::sync::Arc;

mod aot {
    include!("loop_exits_aot.rs");
}

const SOURCE: &str = include_str!("loop_exits.fluxr");
const PREPARED: bool = cfg!(all(
    feature = "typed-jit",
    any(
        target_os = "windows",
        all(target_os = "linux", target_env = "gnu")
    ),
    target_arch = "x86_64"
));
type Outcome = Result<Value, &'static str>;
type Ledger = fn(&[Value], &[Value], Limits, bool) -> Outcome;

const fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn record(total: i64) -> Value {
    Value::Record([Value::I64(total), Value::I64(-11)].into())
}

fn integer(budget: &mut Budget, value: &Value) -> Result<i64, &'static str> {
    budget.cells(1)?;
    match value {
        Value::I64(n) => Ok(*n),
        _ => Err("host value differs from I64"),
    }
}

fn admit_record(budget: &mut Budget, value: &Value) -> Result<i64, &'static str> {
    const WRONG: &str = "host value differs from Record(\"State\")";
    budget.cells(1)?;
    let Value::Record(fields) = value else {
        return Err(WRONG);
    };
    let [total, marker] = &**fields else {
        return Err(WRONG);
    };
    let total = integer(budget, total)?;
    integer(budget, marker)?;
    Ok(total)
}

fn finish(budget: &mut Budget, state: Value) -> Outcome {
    budget.fuel(2)?;
    budget.cells(3)?;
    Ok(state)
}

fn positive(budget: &mut Budget, total: i64, divisor: i64) -> Result<bool, &'static str> {
    budget.fuel(4)?; // Read the field and divisor and dispatch the helper.
    budget.enter(2, 2)?;
    budget.fuel(4)?; // Comparison constant and checked division.
    let quotient = total.checked_div(divisor).ok_or(MATH)?;
    budget.fuel(3)?; // Compare, return, and jump past the short-circuit literal.
    Ok(quotient > 0)
}

const ARITY: &str = "host argument or input count differs from interface";

fn choose(inputs: &[Value], args: &[Value], limits: Limits, subtract: bool) -> Outcome {
    let ([], [state, enabled, left, right]) = (inputs, args) else {
        return Err(ARITY);
    };
    let mut budget = Budget(limits);
    let total = admit_record(&mut budget, state)?;
    budget.cells(1)?;
    let Value::Bool(enabled) = enabled else {
        return Err("host value differs from Bool");
    };
    let left = integer(&mut budget, left)?;
    let right = integer(&mut budget, right)?;
    budget.enter(1, 4)?;
    budget.fuel(2)?; // Read enabled and select the AND path.
    let and = if *enabled {
        positive(&mut budget, total, left)?
    } else {
        budget.fuel(1)?;
        false
    };
    budget.fuel(1)?; // Select the OR path.
    let or = if and {
        budget.fuel(1)?;
        true
    } else {
        positive(&mut budget, total, right)?
    };
    budget.fuel(1)?; // Branch on the OR result.
    let result = if or {
        budget.fuel(4)?;
        let total = if subtract {
            total.checked_sub(1)
        } else {
            total.checked_add(1)
        }
        .ok_or(MATH)?;
        budget.fuel(1)?;
        budget.cells(2)?; // First write detaches the host fields.
        budget.fuel(1)?; // Jump past the empty else arm.
        record(total)
    } else {
        state.clone()
    };
    finish(&mut budget, result)
}

fn walk(inputs: &[Value], args: &[Value], limits: Limits, subtract: bool) -> Outcome {
    let ([], [state, n, stop, skip, step]) = (inputs, args) else {
        return Err(ARITY);
    };
    let mut budget = Budget(limits);
    let initial = admit_record(&mut budget, state)?;
    let mut n = integer(&mut budget, n)?;
    let stop = integer(&mut budget, stop)?;
    let skip = integer(&mut budget, skip)?;
    let step = integer(&mut budget, step)?;
    budget.enter(1, 6)?;
    budget.fuel(3)?; // Initialize the accumulator from the record.
    let mut total = initial;
    loop {
        budget.fuel(4)?; // The loop guard also runs after each backedge.
        if n <= 0 {
            break;
        }
        budget.fuel(4)?; // Decrement and store. The guard makes this safe.
        n -= 1;
        budget.fuel(4)?; // Break test.
        if n == stop {
            budget.fuel(1)?; // Forward jump bypasses the next loop guard.
            break;
        }
        budget.fuel(4)?; // Continue test.
        if n < skip {
            budget.fuel(1)?; // Explicit backedge skips division and the ordinary backedge.
            continue;
        }
        budget.fuel(4)?; // Read total, step and n, then divide.
        let delta = step.checked_div(n).ok_or(MATH)?;
        budget.fuel(1)?;
        total = if subtract {
            total.checked_sub(delta)
        } else {
            total.checked_add(delta)
        }
        .ok_or(MATH)?;
        budget.fuel(2)?; // Store and take the ordinary backedge.
    }
    budget.fuel(5)?; // Compare the accumulator with the host field and branch.
    let result = if total == initial {
        state.clone()
    } else {
        budget.fuel(2)?;
        budget.cells(2)?;
        record(total)
    };
    finish(&mut budget, result)
}

fn choose_args(total: i64, enabled: bool, left: i64, right: i64) -> [Value; 4] {
    [
        record(total),
        Value::Bool(enabled),
        Value::I64(left),
        Value::I64(right),
    ]
}

fn walk_args(total: i64, n: i64, stop: i64, skip: i64, step: i64) -> [Value; 5] {
    [
        record(total),
        Value::I64(n),
        Value::I64(stop),
        Value::I64(skip),
        Value::I64(step),
    ]
}

fn ownership(args: &[Value], expected: &Value, actual: &Value) {
    let (Value::Record(host), Value::Record(want), Value::Record(got)) =
        (&args[0], expected, actual)
    else {
        panic!("successful invocation returns a record");
    };
    assert_eq!(Arc::ptr_eq(host, got), Arc::ptr_eq(host, want));
}

fn malformed(args: &[Value], mut check: impl FnMut(&[Value], &[Value], Limits)) {
    let mut bad_field = args.to_vec();
    bad_field[0] = Value::Record([Value::Unit, Value::I64(-11)].into());
    let mut bad_scalar = args.to_vec();
    bad_scalar[1] = Value::Unit;
    for (inputs, args) in [
        (&[Value::Unit][..], args),
        (&[][..], &args[..2]),
        (&[][..], &bad_field),
        (&[][..], &bad_scalar),
    ] {
        for cells in 0..=7 {
            for fuel in [0, u64::MAX] {
                for frames in 0..=2 {
                    check(inputs, args, limits(fuel, cells, frames));
                }
            }
        }
    }
}

fn check_probe(
    module: &Module,
    export: &str,
    ledger: Ledger,
    change: (&str, &str),
    good: &[Value],
    exact: Limits,
    rejected: &[(&[Value], Limits, &str)],
) {
    let original = module.encode();
    let replacement = Module::compile_compact(&SOURCE.replace(change.0, change.1)).unwrap();
    assert_eq!(
        replacement.prepare_native(export),
        export == "walk" && PREPARED
    );
    let bytes = replacement.encode();
    assert_ne!(bytes, original);
    let mut slot = ModuleSlot::new(module.clone(), &[]).unwrap();
    let mut workspace = Workspace::default();
    let call = |arguments, limits| Invocation {
        export,
        inputs: &[],
        arguments,
        limits,
    };
    let old = ledger(&[], good, exact, false).unwrap();
    let new = ledger(&[], good, exact, true).unwrap();
    assert_ne!(old, new);
    for &(args, budget, trap) in rejected {
        let saved = args.to_vec();
        assert_eq!(ledger(&[], args, budget, true), Err(trap));
        assert_eq!(
            slot.inject(0, replacement.clone(), Some(call(args, budget)))
                .unwrap_err()
                .to_string(),
            trap
        );
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        let actual = slot.run_with(call(good, exact), &mut workspace).unwrap();
        assert_eq!(actual, old);
        ownership(good, &old, &actual);
        assert_eq!(args, saved);
    }
    assert_eq!(
        slot.inject(0, replacement, Some(call(good, exact)))
            .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), bytes);
    let actual = slot.run_with(call(good, exact), &mut workspace).unwrap();
    assert_eq!(actual, new);
    ownership(good, &new, &actual);
}

pub(crate) fn check_loop_exits() -> (usize, usize) {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native("choose"));
    assert_eq!(
        module.emit_native_rust("choose").unwrap_err().to_string(),
        "unsupported native AOT export choose"
    );
    assert_eq!(module.prepare_native("walk"), PREPARED);
    let aot = aot::prepare(&module).unwrap();
    let mut boolean = Replay::new(module.clone(), "choose", &[], &[]);
    let mut loops = Replay::new(module.clone(), "walk", &[], &[]);
    let mut workspace = Workspace::default();
    let mut check_boolean = |inputs: &[Value], args: &[Value], limits| {
        let expected = choose(inputs, args, limits, false);
        boolean.check_routes(inputs, args, limits, expected.clone(), |actual| {
            ownership(args, expected.as_ref().unwrap(), actual);
        });
    };
    for (total, enabled, left, right) in [
        (7, false, 0, -1),
        (7, false, 0, 1),
        (7, false, 1, 0),
        (7, true, 1, 0),
        (7, true, -1, 0),
        (7, true, -1, 1),
        (7, true, 0, 1),
        (i64::MIN, true, -1, 0),
        (i64::MAX, true, 1, 0),
    ] {
        let args = choose_args(total, enabled, left, right);
        for fuel in 0..=35 {
            for cells in 0..=24 {
                for frames in 0..=2 {
                    check_boolean(&[], &args, limits(fuel, cells, frames));
                }
            }
        }
        assert_eq!(args, choose_args(total, enabled, left, right));
    }
    let chosen = choose_args(7, true, 1, 0);
    malformed(&chosen, &mut check_boolean);

    let mut check_loop = |inputs: &[Value], args: &[Value], limits| {
        let expected = walk(inputs, args, limits, false);
        let inspect = |actual: &Value| ownership(args, expected.as_ref().unwrap(), actual);
        loops.check_routes(inputs, args, limits, expected.clone(), inspect);
        let call = Invocation {
            export: "walk",
            inputs,
            arguments: args,
            limits,
        };
        let actual = aot.run_with(call, &mut workspace);
        assert_eq!(
            actual.as_ref().map_err(ToString::to_string),
            expected.as_ref().map_err(ToString::to_string),
            "walk.aot {limits:?}, inputs {inputs:?}, arguments {args:?}"
        );
        if let Ok(value) = &actual {
            inspect(value);
        }
    };
    for (total, n, stop, skip, step) in [
        (7, 0, 0, 0, 0),
        (7, 1, 0, 1, 0),
        (7, 1, -1, 1, 0),
        (7, 1, -1, 0, 0),
        (7, 2, 0, 0, 0),
        (7, 4, 0, 2, 6),
        (i64::MAX, 3, 0, 0, 2),
        (i64::MIN, 2, 0, 0, -1),
        (7, i64::MIN, 0, 0, 0),
    ] {
        let args = walk_args(total, n, stop, skip, step);
        for fuel in 0..=90 {
            for cells in 0..=20 {
                for frames in 0..=1 {
                    check_loop(&[], &args, limits(fuel, cells, frames));
                }
            }
        }
        assert_eq!(args, walk_args(total, n, stop, skip, step));
    }
    let walked = walk_args(7, 4, 0, 2, 6);
    malformed(&walked, &mut check_loop);
    let deep = walk_args(7, 1024, -1, 1024, 0);
    for budget in [
        limits(17_422, 17, 1),
        limits(17_421, 17, 1),
        limits(17_422, 16, 1),
        limits(17_422, 17, 0),
        Limits::default(),
    ] {
        check_loop(&[], &deep, budget);
    }
    check_loop(
        &[],
        &walk_args(7, i64::MAX, -1, i64::MAX, 0),
        limits(90, 20, 1),
    );
    assert_eq!(deep, walk_args(7, 1024, -1, 1024, 0));

    check_probe(
        &module,
        "choose",
        choose,
        ("(+ state.total 1)", "(- state.total 1)"),
        &chosen,
        limits(24, 19, 2),
        &[
            (&chosen, limits(23, 19, 2), super::FUEL),
            (&chosen, limits(24, 18, 2), super::CELLS),
            (&chosen, limits(24, 19, 1), super::FRAMES),
            (&choose_args(7, true, 0, 1), limits(24, 19, 2), MATH),
        ],
    );
    check_probe(
        &module,
        "walk",
        walk,
        ("(+ total (/ step n))", "(- total (/ step n))"),
        &walked,
        limits(88, 19, 1),
        &[
            (&walked, limits(87, 19, 1), super::FUEL),
            (&walked, limits(88, 18, 1), super::CELLS),
            (&walked, limits(88, 19, 0), super::FRAMES),
            (&walk_args(7, 4, -1, 0, 6), limits(92, 19, 1), MATH),
        ],
    );
    assert_eq!(chosen, choose_args(7, true, 1, 0));
    assert_eq!(walked, walk_args(7, 4, 0, 2, 6));
    (boolean.count, loops.count)
}
