//! A source-level contract for one nested record and enum procedure family.
use super::{CELLS, FRAMES, FUEL, MATH, Replay};
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};

const SOURCE: &str = "v2
    (rec Pair x:i64 y:i64)
    (enum Signal None Some:Pair)
    (rec Envelope signal:#Signal point:Pair)
    (inputs hint:#Signal)
    (fn shift (p:Pair d:i64) Pair ()
        (= p.x (+ p.x d))
        (= p.y (/ p.y d))
        (ret p))
    (fn update (s:Envelope hint:#Signal d:i64) Envelope ()
        (ret (new Envelope hint (call shift s.point d))))
    (pub apply (s:Envelope d:i64) Envelope ()
        (ret (call update s (input hint) d)))";

type Expected<T> = Result<T, &'static str>;

struct Budget(Limits);

impl Budget {
    fn fuel(&mut self, amount: u64) -> Expected<()> {
        self.0.instructions = self.0.instructions.checked_sub(amount).ok_or(FUEL)?;
        Ok(())
    }

    fn cells(&mut self, amount: usize) -> Expected<()> {
        self.0.cells = self.0.cells.checked_sub(amount).ok_or(CELLS)?;
        Ok(())
    }

    fn enter(&mut self, depth: usize, slots: usize) -> Expected<()> {
        if depth > self.0.frames {
            return Err(FRAMES);
        }
        self.cells(slots + 1)
    }

    fn integer(&mut self, value: &Value) -> Expected<i64> {
        self.cells(1)?;
        match value {
            Value::I64(n) => Ok(*n),
            _ => Err("host value differs from I64"),
        }
    }

    fn pair(&mut self, value: &Value) -> Expected<(i64, i64)> {
        self.cells(1)?;
        if let Value::Record(fields) = value
            && let [x, y] = &**fields
        {
            return Ok((self.integer(x)?, self.integer(y)?));
        }
        Err("host value differs from Record(\"Pair\")")
    }

    fn signal(&mut self, value: &Value) -> Expected<usize> {
        self.cells(1)?;
        match value {
            Value::Variant(0, None) => Ok(1),
            Value::Variant(1, Some(payload)) => {
                self.pair(payload)?;
                Ok(4)
            }
            Value::Variant(0 | 1, _) => Err("host enum payload differs from interface"),
            Value::Variant(_, _) => Err("unknown host enum tag"),
            _ => Err("host value differs from Enum(\"Signal\")"),
        }
    }
}

// This ledger follows the source evaluation order. The first write detaches
// the two fields from the host's shared point. The second writes the copy.
fn oracle(inputs: &[Value], args: &[Value], limits: Limits, replacement: bool) -> Expected<Value> {
    let ([hint], [state, delta]) = (inputs, args) else {
        return Err("host argument or input count differs from interface");
    };
    let mut budget = Budget(limits);
    let hint_cells = budget.signal(hint)?;
    budget.cells(1)?;
    let Value::Record(fields) = state else {
        return Err("host value differs from Record(\"Envelope\")");
    };
    let [signal, point] = &**fields else {
        return Err("host value differs from Record(\"Envelope\")");
    };
    budget.signal(signal)?;
    let (x, y) = budget.pair(point)?;
    let d = budget.integer(delta)?;

    budget.enter(1, 2)?;
    budget.fuel(4)?; // Three arguments and dispatch to update.
    budget.enter(2, 3)?;
    budget.fuel(5)?; // Read hint, project s.point, read d, dispatch to shift.
    budget.enter(3, 2)?;
    budget.fuel(4)?; // Read p.x and d, then evaluate checked arithmetic.
    let x = if replacement {
        x.checked_sub(d)
    } else {
        x.checked_add(d)
    }
    .ok_or(MATH)?;
    budget.fuel(1)?;
    budget.cells(2)?; // First field write.
    budget.fuel(4)?;
    let y = y.checked_div(d).ok_or(MATH)?;
    budget.fuel(1)?; // Second field write.
    budget.fuel(2)?; // Load and return p.
    budget.fuel(1)?;
    budget.cells(3)?; // Construct Envelope.
    budget.fuel(2)?; // Return from update and apply.
    budget.cells(1 + hint_cells + 3)?; // Result boundary visit.
    Ok(record(hint.clone(), pair(x, y)))
}

fn record(first: Value, second: Value) -> Value {
    Value::Record([first, second].into())
}

fn pair(x: i64, y: i64) -> Value {
    record(Value::I64(x), Value::I64(y))
}

fn some(value: Value) -> Value {
    Value::Variant(1, Some(value.into()))
}

fn arguments(x: i64, y: i64, delta: i64) -> [Value; 2] {
    [record(some(pair(-1, -2)), pair(x, y)), Value::I64(delta)]
}

fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn check_probe(
    module: Module,
    grants: &[Binding],
    inputs: &[Value],
    good: &[Value],
    divide_by_zero: &[Value],
) {
    let original = module.encode();
    let replacement = Module::compile_compact(&SOURCE.replace("(+ p.x d)", "(- p.x d)")).unwrap();
    let replacement_bytes = replacement.encode();
    assert_ne!(replacement_bytes, original);
    let mut slot = ModuleSlot::new(module, grants).unwrap();
    let mut workspace = Workspace::default();
    let full = limits(24, 36, 3);
    let call = |arguments, limits| Invocation {
        export: "apply",
        inputs,
        arguments,
        limits,
    };
    let old = oracle(inputs, good, full, false).unwrap();
    for (args, budget, trap) in [
        (good, limits(17, 36, 3), FUEL),  // After the first write.
        (good, limits(24, 27, 3), CELLS), // After the callee returns.
        (good, limits(24, 36, 2), FRAMES),
        (divide_by_zero, full, MATH), // After the first write.
    ] {
        assert_eq!(oracle(inputs, args, budget, true), Err(trap));
        let candidate = Module::decode(&replacement_bytes).unwrap();
        assert!(!candidate.prepare_native("apply"));
        let error = slot
            .inject(0, candidate, Some(call(args, budget)))
            .unwrap_err();
        assert_eq!(error.to_string(), trap);
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        assert_eq!(
            slot.run_with(call(good, full), &mut workspace).unwrap(),
            old
        );
    }
    assert_eq!(
        slot.inject(0, replacement, Some(call(good, full))).unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), replacement_bytes);
    let new = oracle(inputs, good, full, true).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        slot.run_with(call(good, full), &mut workspace).unwrap(),
        new
    );
}

pub(crate) fn check_compound_effects() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native("apply"));
    let grants = [Binding {
        name: "hint".into(),
        ty: Type::Enum("Signal".into()),
    }];
    let mut replay = Replay::new(module.clone(), "apply", &[], &grants);
    let hint = [some(pair(3, 5))];
    let none = [Value::Variant(0, None)];
    let good = arguments(7, 21, 3);
    let divide_by_zero = arguments(7, 21, 0);
    let full = limits(24, 36, 3);
    let answer = record(hint[0].clone(), pair(10, 7));
    assert_eq!(oracle(&hint, &good, full, false), Ok(answer.clone()));
    for (inputs, args) in [
        (&hint[..], good.clone()),
        (&none, good.clone()),
        (&hint, arguments(i64::MAX, 21, 1)),
        (&hint, arguments(i64::MIN, 21, -1)),
        (&hint, divide_by_zero.clone()),
        (&hint, arguments(7, i64::MIN, -1)),
    ] {
        for cells in 0..=37 {
            for fuel in 0..=25 {
                for frames in 0..=3 {
                    let budget = limits(fuel, cells, frames);
                    replay.check_call(inputs, &args, budget, oracle(inputs, &args, budget, false));
                }
            }
        }
    }

    // Cell cutoffs around each malformed node. The input is visited before
    // arguments, including when the snapshot and the arguments are both bad.
    for (inputs, args) in [
        (&[][..], &good[..]),
        (&hint, &good[..1]),
        (&[Value::Variant(9, None)], &good),
        (&[Value::Variant(1, None)], &good),
        (&[Value::Variant(0, Some(Value::Unit.into()))], &good),
        (&[some(Value::Record([Value::I64(1)].into()))], &good),
        (
            &[some(record(Value::I64(1), Value::Bool(false)))],
            &[Value::Unit, Value::Unit],
        ),
        (&hint, &[record(Value::Unit, pair(1, 2)), Value::I64(3)]),
        (&hint, &[good[0].clone(), Value::Bool(false)]),
    ] {
        for cells in 0..=14 {
            for frames in [0, 3] {
                for fuel in [0, 24] {
                    let budget = limits(fuel, cells, frames);
                    replay.check_call(inputs, args, budget, oracle(inputs, args, budget, false));
                }
            }
        }
    }

    check_probe(module, &grants, &hint, &good, &divide_by_zero);
    assert_eq!(good, arguments(7, 21, 3));
    assert_eq!(divide_by_zero, arguments(7, 21, 0));
    assert_eq!(hint, [some(pair(3, 5))]);
    // The last call on the reused workspace follows both trap and budget exits.
    replay.check_call(&hint, &good, full, Ok(answer));
    replay.count
}
