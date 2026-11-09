//! Source-level effect ledger for an indexed row edited through a shared list.
use super::{Budget, CELLS, FRAMES, FUEL, Replay, list_effect_checks::list};
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value};
use std::sync::Arc;

const SOURCE: &str = include_str!("list_boundary.fluxr");
const EXPORT: &str = "edit";
const BOUNDS: &str = "list index out of bounds";
type Expected<T> = Result<T, &'static str>;

fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn rows(budget: &mut Budget, value: &Value) -> Expected<Vec<Vec<i64>>> {
    budget.cells(1)?;
    let Value::List(rows) = value else {
        return Err("host value differs from List(List(I64))");
    };
    rows.iter()
        .map(|row| {
            budget.cells(1)?;
            let Value::List(values) = row else {
                return Err("host value differs from List(I64)");
            };
            values
                .iter()
                .map(|value| {
                    budget.cells(1)?;
                    match value {
                        Value::I64(n) => Ok(*n),
                        _ => Err("host value differs from I64"),
                    }
                })
                .collect()
        })
        .collect()
}

fn index(budget: &mut Budget, value: &Value) -> Expected<u64> {
    budget.cells(1)?;
    match value {
        Value::U64(n) => Ok(*n),
        _ => Err("host value differs from U64"),
    }
}

fn at<T>(list: &[T], index: u64) -> Expected<&T> {
    usize::try_from(index)
        .ok()
        .and_then(|index| list.get(index))
        .ok_or(BOUNDS)
}

fn ledger(inputs: &[Value], args: &[Value], limits: Limits, edited: bool) -> Expected<Value> {
    let ([base], [source, row, col]) = (inputs, args) else {
        return Err("host argument or input count differs from interface");
    };
    let mut budget = Budget(limits);
    // Visit both trees in order, even when the roots or their children alias.
    let base = rows(&mut budget, base)?;
    let source = rows(&mut budget, source)?;
    let row = index(&mut budget, row)?;
    let col = index(&mut budget, col)?;
    budget.enter(1, 3)?;
    budget.fuel(3)?; // Read rows and row, then select the row.
    let original = at(&source, row)?;
    budget.fuel(3)?; // Read row and col, then dispatch.
    budget.enter(2, 3)?;
    budget.fuel(2)?; // Load 7 and push, detaching the row held by the host.
    budget.cells(original.len() + 1)?;
    budget.fuel(1)?; // Pop 7 from the private list.
    budget.fuel(3)?; // Read the snapshot and row, then select its row.
    let snapshot = at(&base, row)?;
    budget.fuel(2)?; // Read the column and select its element.
    let value = *at(snapshot, if edited { 0 } else { col })?;
    budget.fuel(1)?;
    budget.cells(1)?; // Push to the private list without another copy.
    budget.fuel(3)?; // Load the result, return from patch, return from edit.
    budget.cells(original.len() + 2)?; // Admit the result root and elements.
    Ok(list(
        original.iter().copied().chain([value]).map(Value::I64),
    ))
}

fn row(values: &[i64]) -> Value {
    list(values.iter().copied().map(Value::I64))
}

fn arguments(rows: Value, row: u64, col: u64) -> [Value; 3] {
    [rows, Value::U64(row), Value::U64(col)]
}

fn call<'a>(inputs: &'a [Value], arguments: &'a [Value], limits: Limits) -> Invocation<'a> {
    Invocation {
        export: EXPORT,
        inputs,
        arguments,
        limits,
    }
}

fn check_replacement(module: Module, grants: &[Binding], inputs: &[Value], args: &[Value]) {
    let original = module.encode();
    let mut slot = ModuleSlot::new(module, grants).unwrap();
    let source = SOURCE.replace("base) row) col)", "base) row) 0u)");
    let replacement = Module::compile_compact(&source).unwrap();
    let bytes = replacement.encode();
    assert_ne!(original, bytes);
    let full = limits(18, 32, 2);
    let mut workspace = Default::default();
    let old = ledger(inputs, args, full, false).unwrap();
    let new = ledger(inputs, args, full, true).unwrap();
    assert_ne!(old, new);
    let missing_row = arguments(list([row(&[]), row(&[]), row(&[5, 9])]), 2, 1);
    for (probe, budget, trap) in [
        (args, limits(6, 32, 1), FRAMES),
        (args, limits(8, 32, 2), FUEL),   // After the first push.
        (args, limits(18, 31, 2), CELLS), // After both pushes, at result admission.
        (&missing_row[..], limits(18, 40, 2), BOUNDS), // After push and pop.
    ] {
        assert_eq!(ledger(inputs, probe, budget, true), Err(trap));
        let error = slot
            .inject(
                0,
                Module::decode(&bytes).unwrap(),
                Some(call(inputs, probe, budget)),
            )
            .unwrap_err();
        assert_eq!(error.to_string(), trap);
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        assert_eq!(
            slot.run_with(call(inputs, args, full), &mut workspace)
                .unwrap(),
            old
        );
    }
    assert_eq!(
        slot.inject(0, replacement, Some(call(inputs, args, full)))
            .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), bytes);
    assert_eq!(
        slot.run_with(call(inputs, args, full), &mut workspace)
            .unwrap(),
        new
    );
}

pub(crate) fn check_list_boundary() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native(EXPORT));
    let grants = [Binding {
        name: "base".into(),
        ty: Type::List(Box::new(Type::List(Box::new(Type::I64)))),
    }];
    let mut replay = Replay::new(module.clone(), EXPORT, &[], &grants);
    let mut source = ModuleSlot::new(module.clone(), &grants).unwrap();
    let compiled = Module::compile_compact(SOURCE).unwrap();
    assert_eq!(source.inject(0, compiled, None).unwrap(), 1);
    let mut workspace = Default::default();
    let first = row(&[5, 9]);
    let matrix = list([first.clone(), first.clone()]);
    let base = [matrix.clone()];
    let good = arguments(matrix.clone(), 1, 1);
    let full = limits(18, 32, 2);
    let answer = row(&[5, 9, 9]);
    let Value::List(shared) = &first else {
        unreachable!();
    };
    let references = Arc::strong_count(shared);
    assert_eq!(ledger(&base, &good, full, false), Ok(answer.clone()));
    for (inputs, args) in [
        (&base[..], &good[..]),
        (&base, &arguments(matrix.clone(), 2, 1)),
        (&base, &arguments(matrix.clone(), 1, 2)),
        (&base, &arguments(matrix.clone(), 0, 1 << 32)),
        (&base, &arguments(matrix.clone(), 0, u64::MAX)),
        (&base, &arguments(matrix.clone(), 1 << 32, 0)),
        (&base, &arguments(list([row(&[])]), 0, 1)),
        (&[list([row(&[])])], &good),
        (&[list([row(&[]), row(&[])])], &good),
        (&[Value::Unit], &good),
        (&[list([first.clone(), Value::Unit])], &good),
        (
            &[list([first.clone(), list([Value::I64(5), Value::Unit])])],
            &good,
        ),
        (
            &base,
            &arguments(list([first.clone(), list([Value::Unit])]), 0, 1),
        ),
        (&base, &[matrix.clone(), Value::U64(1), Value::I64(1)]),
        (&[Value::Unit], &good[..2]),
        (&[], &good),
    ] {
        for cells in 0..=34 {
            for fuel in 0..=19 {
                for frames in 0..=2 {
                    let budget = limits(fuel, cells, frames);
                    let expected = ledger(inputs, args, budget, false);
                    replay.check_call(inputs, args, budget, expected.clone());
                    let actual = source
                        .run_with(call(inputs, args, budget), &mut workspace)
                        .map_err(|error| error.to_string());
                    assert_eq!(
                        actual.as_ref().map_err(String::as_str),
                        expected.as_ref().map_err(|&error| error),
                        "source injection {budget:?}, inputs {inputs:?}, arguments {args:?}"
                    );
                }
            }
        }
    }

    // Both slots and the reused workspaces can run after the trap grid.
    let detached = |value: &Value| {
        let Value::List(result) = value else {
            unreachable!();
        };
        assert!(!Arc::ptr_eq(shared, result));
    };
    replay.check_routes(&base, &good, full, Ok(answer.clone()), detached);
    let result = source
        .run_with(call(&base, &good, full), &mut workspace)
        .unwrap();
    assert_eq!(result, answer);
    detached(&result);
    check_replacement(module, &grants, &base, &good);
    assert_eq!(base, [list([row(&[5, 9]), row(&[5, 9])])]);
    assert_eq!(good, arguments(list([row(&[5, 9]), row(&[5, 9])]), 1, 1));
    assert_eq!(first, row(&[5, 9]));
    assert_eq!(Arc::strong_count(shared), references);
    replay.count
}
