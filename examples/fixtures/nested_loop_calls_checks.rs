//! Source ledger for signed nested loop exits and the conditionally reached call.
use super::{Budget, CELLS, FRAMES, FUEL, MATH, Replay};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Value, Workspace};
use std::sync::Arc;

pub(crate) const SOURCE: &str = include_str!("nested_loop_calls.fluxr");
const PREPARED: bool = cfg!(all(
    feature = "typed-jit",
    any(
        target_os = "windows",
        all(target_os = "linux", target_env = "gnu")
    ),
    target_arch = "x86_64"
));
const ARITY: &str = "host argument or input count differs from interface";
const RECORD: &str = "host value differs from Record(\"State\")";

mod aot {
    include!("nested_loop_calls_aot.rs");
}

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

fn arguments(total: i64, scalars: [i64; 5]) -> [Value; 6] {
    let [n, width, trigger, step, divisor] = scalars.map(Value::I64);
    [record(total), n, width, trigger, step, divisor]
}

fn integer(budget: &mut Budget, value: &Value) -> Result<i64, &'static str> {
    budget.cells(1)?;
    match value {
        Value::I64(n) => Ok(*n),
        _ => Err("host value differs from I64"),
    }
}

fn walk(
    inputs: &[Value],
    args: &[Value],
    limits: Limits,
    subtract: bool,
) -> Result<Value, &'static str> {
    let ([], [state, n, width, trigger, step, divisor]) = (inputs, args) else {
        return Err(ARITY);
    };
    let mut budget = Budget(limits);
    budget.cells(1)?;
    let Value::Record(fields) = state else {
        return Err(RECORD);
    };
    let [total, marker] = &**fields else {
        return Err(RECORD);
    };
    let initial = integer(&mut budget, total)?;
    let marker = integer(&mut budget, marker)?;
    let mut n = integer(&mut budget, n)?;
    let width = integer(&mut budget, width)?;
    let trigger = integer(&mut budget, trigger)?;
    let step = integer(&mut budget, step)?;
    let divisor = integer(&mut budget, divisor)?;
    budget.enter(1, 8)?;
    budget.fuel(3)?; // Initialize the accumulator from the record.
    let mut total = initial;
    loop {
        budget.fuel(4)?;
        if n <= 0 {
            break;
        }
        budget.fuel(4)?; // Decrement and store.
        n -= 1;
        budget.fuel(4)?;
        if n == 0 {
            budget.fuel(1)?; // Outer break skips the inner loop and next outer guard.
            break;
        }
        budget.fuel(4)?;
        if n == 2 {
            budget.fuel(1)?; // Outer continue skips the inner loop and outer addition.
            continue;
        }
        budget.fuel(2)?; // Initialize the inner counter.
        let mut j = width;
        loop {
            budget.fuel(4)?;
            if j <= 0 {
                break;
            }
            budget.fuel(4)?;
            j -= 1;
            budget.fuel(4)?;
            if j == 1 {
                budget.fuel(1)?; // Inner break still reaches the outer addition.
                break;
            }
            budget.fuel(4)?;
            if j == 3 {
                budget.fuel(1)?; // Inner continue reaches the next inner guard.
                continue;
            }
            budget.fuel(4)?; // Test whether to reach the helper.
            if j == trigger {
                budget.fuel(4)?; // Read accumulator and helper arguments, then dispatch.
                budget.enter(2, 2)?;
                budget.fuel(3)?; // Load both operands and divide.
                let delta = step.checked_div(divisor).ok_or(MATH)?;
                budget.fuel(2)?; // Return, then add or subtract in the caller.
                total = if subtract {
                    total.checked_sub(delta)
                } else {
                    total.checked_add(delta)
                }
                .ok_or(MATH)?;
                budget.fuel(2)?; // Store and jump past the empty else arm.
            }
            budget.fuel(1)?; // Ordinary inner backedge.
        }
        budget.fuel(3)?;
        total = total.checked_add(n).ok_or(MATH)?;
        budget.fuel(2)?; // Store and take the outer backedge.
    }
    budget.fuel(5)?; // Compare the accumulator with the original field and branch.
    let result = if total == initial {
        state.clone()
    } else {
        budget.fuel(2)?;
        budget.cells(2)?; // First write detaches the host record.
        Value::Record([Value::I64(total), Value::I64(marker)].into())
    };
    budget.fuel(2)?;
    budget.cells(3)?;
    Ok(result)
}

fn ownership(args: &[Value], expected: &Value, actual: &Value) {
    let (Value::Record(host), Value::Record(want), Value::Record(got)) =
        (&args[0], expected, actual)
    else {
        panic!("successful invocation returns a record");
    };
    assert_eq!(Arc::ptr_eq(host, got), Arc::ptr_eq(host, want));
}

fn call(args: &[Value], limits: Limits) -> Invocation<'_> {
    Invocation {
        export: "walk",
        inputs: &[],
        arguments: args,
        limits,
    }
}

fn check_probes(module: &Module) {
    let replacement =
        Module::compile_compact(&SOURCE.replace("(+ total (call", "(- total (call")).unwrap();
    assert_eq!(replacement.prepare_native("walk"), PREPARED);
    let bytes = module.encode();
    let candidate_bytes = replacement.encode();
    assert_ne!(candidate_bytes, bytes);
    let mut slot = ModuleSlot::new(module.clone(), &[]).unwrap();
    let mut workspace = Workspace::default();
    let good = arguments(7, [2, 4, 2, 6, 2]);
    let div_zero = arguments(7, [2, 4, 2, 6, 0]);
    let exact = limits(110, 25, 2);
    let old = walk(&[], &good, exact, false).unwrap();
    let new = walk(&[], &good, exact, true).unwrap();
    assert_eq!(old, record(11));
    assert_eq!(new, record(5));
    for (args, budget, trap) in [
        (&good, limits(109, 25, 2), FUEL),
        (&good, limits(110, 24, 2), CELLS),
        (&good, limits(110, 25, 1), FRAMES),
        (&div_zero, exact, MATH),
    ] {
        let saved = args.to_vec();
        assert_eq!(walk(&[], args, budget, true), Err(trap));
        assert_eq!(
            slot.inject(0, replacement.clone(), Some(call(args, budget)))
                .unwrap_err()
                .to_string(),
            trap
        );
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), bytes);
        let actual = slot.run_with(call(&good, exact), &mut workspace).unwrap();
        assert_eq!(actual, old);
        ownership(&good, &old, &actual);
        assert_eq!(args, saved.as_slice());
    }
    assert_eq!(
        slot.inject(0, replacement, Some(call(&good, exact)))
            .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), candidate_bytes);
    let actual = slot.run_with(call(&good, exact), &mut workspace).unwrap();
    assert_eq!(actual, new);
    ownership(&good, &new, &actual);
    assert_eq!(good, arguments(7, [2, 4, 2, 6, 2]));
    assert_eq!(div_zero, arguments(7, [2, 4, 2, 6, 0]));
}

pub(crate) fn check_nested_loop_calls() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert_eq!(module.prepare_native("walk"), PREPARED);
    let aot = aot::prepare(&module).unwrap();
    let mut replay = Replay::new(module.clone(), "walk", &[], &[]);
    let mut workspace = Workspace::default();
    let mut check = |inputs: &[Value], args: &[Value], limits| {
        let expected = walk(inputs, args, limits, false);
        let inspect = |actual: &Value| ownership(args, expected.as_ref().unwrap(), actual);
        replay.check_routes(inputs, args, limits, expected.clone(), inspect);
        let actual = aot.run_with(
            Invocation {
                export: "walk",
                inputs,
                arguments: args,
                limits,
            },
            &mut workspace,
        );
        assert_eq!(
            actual.as_ref().map_err(ToString::to_string),
            expected.as_ref().map_err(ToString::to_string),
            "walk.aot {limits:?}, inputs {inputs:?}, arguments {args:?}"
        );
        if let Ok(value) = &actual {
            inspect(value);
        }
    };
    for (total, scalars, fuel, cells, frames, result) in [
        (7, [0, 4, 2, 6, 0], 14, 20, 1, 7),   // No outer body.
        (7, [1, 4, 2, 6, 0], 23, 20, 1, 7),   // Outer break.
        (7, [3, 0, 2, 6, 0], 69, 22, 1, 8),   // Outer continue.
        (7, [2, 4, 3, 6, 0], 99, 22, 1, 8),   // Inner continue skips matching trigger.
        (7, [2, 4, 1, 6, 0], 99, 22, 1, 8),   // Inner break skips matching trigger.
        (7, [2, 1, 0, 6, 2], 84, 25, 2, 11),  // Inner guard exit after call.
        (7, [2, 4, 2, 6, 2], 110, 25, 2, 11), // Both inner exits before the outer break.
        (7, [2, 4, 2, -1, 1], 108, 23, 2, 7), // Arithmetic cancels, preserving the allocation.
        (7, [4, 4, 2, 0, 1], 212, 28, 2, 11), // Two calls separated by the outer continue.
        (7, [1024, 4, 2, 0, 1], 86_912, 3088, 2, 523_781),
    ] {
        let args = arguments(total, scalars);
        let exact = limits(fuel, cells, frames);
        assert_eq!(
            walk(&[], &args, exact, false),
            Ok(record(result)),
            "{scalars:?}"
        );
        for (budget, trap) in [
            (limits(fuel - 1, cells, frames), FUEL),
            (limits(fuel, cells - 1, frames), CELLS),
            (limits(fuel, cells, frames - 1), FRAMES),
        ] {
            assert_eq!(walk(&[], &args, budget, false), Err(trap), "{scalars:?}");
            check(&[], &args, budget);
        }
        check(&[], &args, exact);
        // Native admission reserves the graph's maximum frame depth.
        check(&[], &args, limits(fuel, cells, 2));
        assert_eq!(args, arguments(total, scalars));
    }
    for (total, scalars) in [
        (7, [2, 4, 2, 6, 0]),
        (7, [2, 4, 2, i64::MIN, -1]),
        (i64::MAX, [2, 4, 2, 1, 1]),
        (i64::MAX, [2, 4, 3, 6, 0]),
        (7, [3, 4, 2, 6, 2]),
    ] {
        let args = arguments(total, scalars);
        for fuel in 0..=128 {
            for cells in 0..=26 {
                for frames in 0..=2 {
                    check(&[], &args, limits(fuel, cells, frames));
                }
            }
        }
        assert_eq!(args, arguments(total, scalars));
    }
    let good = arguments(7, [2, 4, 2, 6, 2]);
    let mut bad_field = good.clone();
    bad_field[0] = Value::Record([Value::I64(7), Value::Unit].into());
    let mut bad_scalar = good.clone();
    bad_scalar[5] = Value::Bool(false);
    for (inputs, args) in [
        (&[Value::Unit][..], &good[..]),
        (&[][..], &good[..5]),
        (&[][..], &bad_field),
        (&[][..], &bad_scalar),
    ] {
        for cells in 0..=8 {
            for fuel in [0, 212] {
                for frames in 0..=2 {
                    check(inputs, args, limits(fuel, cells, frames));
                }
            }
        }
    }
    check_probes(&module);
    replay.count
}
