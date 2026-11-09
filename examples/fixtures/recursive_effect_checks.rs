//! Source-level effects of a recursive signed fold.
use super::{Budget, CELLS, FRAMES, FUEL, MATH, Replay};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Value, Workspace};

const EXPORT: &str = "fold";
const SOURCE: &str = "v2
    (pub fold (n:i64 seed:i64) i64 ()
        (if (<= n 0) ((ret seed)) ((ret (+ n (call fold (- n 1) seed))))))";

fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn oracle(
    inputs: &[Value],
    args: &[Value],
    limits: Limits,
    replacement: bool,
) -> Result<Value, &'static str> {
    let ([], [n, seed]) = (inputs, args) else {
        return Err("host argument or input count differs from interface");
    };
    let mut budget = Budget(limits);
    let mut admit = |value: &Value| {
        budget.cells(1)?;
        match value {
            Value::I64(n) => Ok(*n),
            _ => Err("host value differs from I64"),
        }
    };
    let mut n = admit(n)?;
    let mut result = admit(seed)?;
    let start = n;
    let mut depth = 1;
    loop {
        budget.enter(depth, 2)?;
        budget.fuel(4)?; // Two comparison operands, comparison and branch.
        if n <= 0 {
            budget.fuel(2)?; // Read the seed and return.
            break;
        }
        // Hold n across the call, evaluate n - 1 and seed, then dispatch.
        budget.fuel(6)?;
        n -= 1; // The branch guarantees the source subtraction is safe.
        depth += 1;
    }
    // Each suspended caller combines its retained n with the returned value.
    for caller in 1..=start {
        budget.fuel(1)?;
        result = if replacement {
            caller.checked_sub(result)
        } else {
            caller.checked_add(result)
        }
        .ok_or(MATH)?;
        budget.fuel(1)?;
    }
    budget.cells(1)?; // Admit the returned scalar only after the outer return.
    Ok(Value::I64(result))
}

fn check_probe(module: Module, good: &[Value], exact: Limits) {
    let original = module.encode();
    let replacement = Module::compile_compact(&SOURCE.replace("(+ n (call", "(- n (call"))
        .unwrap()
        .encode();
    assert_ne!(original, replacement);
    let mut slot = ModuleSlot::new(module, &[]).unwrap();
    let mut workspace = Workspace::default();
    let call = |arguments, limits| Invocation {
        export: EXPORT,
        inputs: &[],
        arguments,
        limits,
    };
    let old = oracle(&[], good, exact, false).unwrap();
    for (args, budget, trap) in [
        (good, limits(40, 15, 4), FUEL),
        (good, limits(42, 14, 4), CELLS),
        (good, limits(42, 15, 3), FRAMES),
        (&[Value::I64(1), Value::I64(i64::MIN)], exact, MATH),
    ] {
        assert_eq!(oracle(&[], args, budget, true), Err(trap));
        let candidate = Module::decode(&replacement).unwrap();
        assert!(!candidate.prepare_native(EXPORT));
        let error = slot
            .inject(0, candidate, Some(call(args, budget)))
            .unwrap_err();
        assert_eq!(error.to_string(), trap);
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        assert_eq!(
            slot.run_with(call(good, exact), &mut workspace).unwrap(),
            old
        );
    }
    let candidate = Module::decode(&replacement).unwrap();
    assert_eq!(
        slot.inject(0, candidate, Some(call(good, exact))).unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), replacement);
    let new = oracle(&[], good, exact, true).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        slot.run_with(call(good, exact), &mut workspace).unwrap(),
        new
    );
}

pub(crate) fn check_recursive_effects() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native(EXPORT));
    assert!(module.native_ceiling(EXPORT, 2).is_none());
    assert!(module.native_loop_ceiling(EXPORT, 2, 1024).is_none());
    let mut replay = Replay::new(module.clone(), EXPORT, &[], &[]);
    let good = [Value::I64(3), Value::I64(7)];
    let exact = limits(42, 15, 4);
    let answer = Value::I64(13);
    assert_eq!(oracle(&[], &good, exact, false), Ok(answer.clone()));
    for (n, seed) in [
        (0, 7),
        (i64::MIN, 7),
        (1, 7),
        (3, 7),
        (3, i64::MAX),
        (3, i64::MAX - 3),
    ] {
        let args = [Value::I64(n), Value::I64(seed)];
        for cells in 0..=16 {
            for fuel in 0..=43 {
                for frames in 0..=5 {
                    let budget = limits(fuel, cells, frames);
                    replay.check(&args, budget, oracle(&[], &args, budget, false));
                }
            }
        }
    }
    for (inputs, args) in [
        (&[Value::Unit][..], &good[..]),
        (&[][..], &good[..1]),
        (&[][..], &[Value::Bool(false), Value::Unit]),
        (&[][..], &[Value::I64(3), Value::Bool(false)]),
    ] {
        for cells in 0..=4 {
            for fuel in [0, 42] {
                for frames in [0, 4] {
                    let budget = limits(fuel, cells, frames);
                    replay.check_call(inputs, args, budget, oracle(inputs, args, budget, false));
                }
            }
        }
    }
    let deep = [Value::I64(1024), Value::I64(0)];
    for budget in [
        limits(12_294, 3078, 1025),
        limits(12_293, 3078, 1025),
        limits(12_294, 3077, 1025),
        limits(12_294, 3078, 1024),
    ] {
        replay.check(&deep, budget, oracle(&[], &deep, budget, false));
    }
    let huge = [Value::I64(i64::MAX), Value::I64(0)];
    replay.check(&huge, exact, Err(FRAMES));
    check_probe(module, &good, exact);
    assert_eq!(good, [Value::I64(3), Value::I64(7)]);
    replay.check(&good, exact, Ok(answer));
    replay.count
}
