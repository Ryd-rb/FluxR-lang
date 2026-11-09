//! Source-level ledger for a guarded extraction followed by a shared-list edit.
use super::{Budget, CELLS, FRAMES, FUEL, Replay, list_effect_checks::list};
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value};
use std::sync::Arc;

const SOURCE: &str = include_str!("variant_payload.fluxr");
const EXPORT: &str = "edit";
const WRONG_VARIANT: &str = "enum payload does not match variant";
const BAD_PAYLOAD: &str = "host enum payload differs from interface";
type Expected<T> = Result<T, &'static str>;

fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn admit<'a>(budget: &mut Budget, value: &'a Value) -> Expected<(usize, &'a [Value])> {
    budget.cells(1)?;
    let Value::Variant(tag, payload) = value else {
        return Err("host value differs from Enum(\"Choice\")");
    };
    match (*tag, payload.as_deref()) {
        (0, None) => Ok((0, &[])),
        (1 | 2, Some(value)) => {
            budget.cells(1)?;
            let Value::List(values) = value else {
                return Err("host value differs from List(I64)");
            };
            for value in values.iter() {
                budget.cells(1)?;
                if !matches!(value, Value::I64(_)) {
                    return Err("host value differs from I64");
                }
            }
            Ok((*tag, values))
        }
        (0..=2, _) => Err(BAD_PAYLOAD),
        _ => Err("unknown host enum tag"),
    }
}

fn ledger(inputs: &[Value], args: &[Value], limits: Limits, base_tag: usize) -> Expected<Value> {
    let ([base], [selected, force]) = (inputs, args) else {
        return Err("host argument or input count differs from interface");
    };
    let mut budget = Budget(limits);
    let (snapshot_tag, snapshot) = admit(&mut budget, base)?;
    let (tag, row) = admit(&mut budget, selected)?;
    budget.cells(1)?;
    let Value::Bool(force) = force else {
        return Err("host value differs from Bool");
    };
    budget.enter(1, 2)?;
    budget.fuel(3)?; // Read selected, test the tag, branch.
    if tag != 1 {
        budget.fuel(2)?; // Read force and branch.
        if !force {
            budget.fuel(1)?;
            budget.cells(1)?; // Allocate the empty list.
            budget.fuel(1)?; // Return.
            budget.cells(1)?; // Admit the result.
            return Ok(list([]));
        }
    }
    budget.fuel(2)?; // Read selected and dispatch.
    budget.enter(2, 2)?;
    budget.fuel(2)?; // Read selected and extract its payload.
    if tag != 1 {
        return Err(WRONG_VARIANT);
    }
    budget.fuel(3)?; // Store row, read 7, push.
    budget.cells(row.len() + 1)?; // Detach the shared payload on the first write.
    budget.fuel(2)?; // Read base and extract its payload after the first push.
    if snapshot_tag != base_tag {
        return Err(WRONG_VARIANT);
    }
    budget.fuel(2)?; // Read index and look up the original value.
    let first = snapshot.first().ok_or("list index out of bounds")?;
    budget.fuel(1)?;
    budget.cells(1)?; // Push into the private list.
    budget.fuel(3)?; // Read row and return through both frames.
    budget.cells(row.len() + 3)?; // Admit the list and all its elements.
    Ok(list(
        row.iter().cloned().chain([Value::I64(7), first.clone()]),
    ))
}

fn call<'a>(inputs: &'a [Value], arguments: &'a [Value], limits: Limits) -> Invocation<'a> {
    Invocation {
        export: EXPORT,
        inputs,
        arguments,
        limits,
    }
}

fn check_replacement(
    module: Module,
    grants: &[Binding],
    base: &[Value],
    other_base: &[Value],
    args: &[Value],
) {
    let original = module.encode();
    let mut slot = ModuleSlot::new(module, grants).unwrap();
    let source = SOURCE.replace(
        "(payload (input base) Edit)",
        "(payload (input base) Other)",
    );
    let replacement = Module::compile_compact(&source).unwrap().encode();
    assert_ne!(replacement, original);
    let full = limits(18, 24, 2);
    let expected = ledger(base, args, full, 1).unwrap();
    assert_eq!(ledger(base, args, full, 2), Err(WRONG_VARIANT));
    let mut workspace = Default::default();
    for (budget, trap) in [
        (limits(7, 9, 0), FRAMES),
        (limits(11, 24, 2), FUEL),
        (limits(12, 17, 2), CELLS),
        (limits(12, 18, 2), WRONG_VARIANT),
    ] {
        assert_eq!(ledger(base, args, budget, 2), Err(trap));
        let err = slot
            .inject(
                0,
                Module::decode(&replacement).unwrap(),
                Some(call(base, args, budget)),
            )
            .unwrap_err();
        assert_eq!(err.to_string(), trap);
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        assert_eq!(
            slot.run_with(call(base, args, full), &mut workspace)
                .unwrap(),
            expected
        );
    }
    assert_eq!(ledger(other_base, args, full, 1), Err(WRONG_VARIANT));
    assert_eq!(ledger(other_base, args, full, 2), Ok(expected.clone()));
    assert_eq!(
        slot.inject(
            0,
            Module::decode(&replacement).unwrap(),
            Some(call(other_base, args, full)),
        )
        .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), replacement);
    assert_eq!(
        slot.run_with(call(other_base, args, full), &mut workspace)
            .unwrap(),
        expected
    );
}

pub(crate) fn check_variant_payload() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native(EXPORT));
    let grants = [Binding {
        name: "base".into(),
        ty: Type::Enum("Choice".into()),
    }];
    let mut replay = Replay::new(module.clone(), EXPORT, &[], &grants);
    let mut source = ModuleSlot::new(module.clone(), &grants).unwrap();
    assert_eq!(
        source
            .inject(0, Module::compile_compact(SOURCE).unwrap(), None)
            .unwrap(),
        1
    );
    let mut workspace = Default::default();
    let row = list([Value::I64(5), Value::I64(9)]);
    let payload = Arc::new(row.clone());
    let edit = Value::Variant(1, Some(payload.clone()));
    let other = Value::Variant(2, Some(payload.clone()));
    let empty = Value::Variant(0, None);
    let empty_edit = Value::Variant(1, Some(Arc::new(list([]))));
    let bad_element = Value::Variant(1, Some(Arc::new(list([Value::I64(5), Value::Unit]))));
    let missing = Value::Variant(1, None);
    let base = [edit.clone()];
    let other_base = [other.clone()];
    let args = [edit.clone(), Value::Bool(false)];
    let payload_refs = Arc::strong_count(&payload);
    let Value::List(shared) = &row else {
        unreachable!();
    };
    let list_refs = Arc::strong_count(shared);

    for (inputs, arguments) in [
        (&base[..], &args[..]),
        (&base, &[empty.clone(), Value::Bool(false)]),
        (&base, &[other.clone(), Value::Bool(false)]),
        (&base, &[empty.clone(), Value::Bool(true)]),
        (&base, &[other.clone(), Value::Bool(true)]),
        (&other_base, &args),
        (&[empty], &args),
        (&[empty_edit.clone()], &args),
        (&base, &[empty_edit, Value::Bool(false)]),
        (&[Value::Unit], &args),
        (&[Value::Variant(usize::MAX, None)], &args),
        (&[Value::Variant(0, Some(payload.clone()))], &args),
        (&base, &[missing.clone(), Value::Bool(false)]),
        (
            &base,
            &[
                Value::Variant(2, Some(Arc::new(Value::I64(5)))),
                Value::Bool(false),
            ],
        ),
        (&[bad_element.clone()], &[missing, Value::Bool(false)]),
        (&base, &[bad_element, Value::Bool(false)]),
        (&base, &[edit.clone(), Value::Unit]),
        (&[Value::Unit], &args[..1]),
    ] {
        for cells in 0..=26 {
            for fuel in 0..=20 {
                for frames in 0..=2 {
                    let budget = limits(fuel, cells, frames);
                    let expected = ledger(inputs, arguments, budget, 1);
                    replay.check_call(inputs, arguments, budget, expected.clone());
                    let actual = source
                        .run_with(call(inputs, arguments, budget), &mut workspace)
                        .map_err(|error| error.to_string());
                    assert_eq!(
                        actual.as_ref().map_err(String::as_str),
                        expected.as_ref().map_err(|&trap| trap),
                        "source injection {budget:?}, inputs {inputs:?}, arguments {arguments:?}"
                    );
                }
            }
        }
    }
    let full = limits(18, 24, 2);
    let answer = list([5, 9, 7, 5].map(Value::I64));
    let detached = |value: &Value| {
        let Value::List(result) = value else {
            unreachable!();
        };
        assert!(!Arc::ptr_eq(shared, result));
    };
    assert_eq!(ledger(&base, &args, full, 1), Ok(answer.clone()));
    replay.check_routes(&base, &args, full, Ok(answer.clone()), detached);
    let result = source
        .run_with(call(&base, &args, full), &mut workspace)
        .unwrap();
    assert_eq!(result, answer);
    detached(&result);
    check_replacement(module, &grants, &base, &other_base, &args);

    let original = Arc::new(list([5, 9].map(Value::I64)));
    assert_eq!(base, [Value::Variant(1, Some(original.clone()))]);
    assert_eq!(
        args,
        [
            Value::Variant(1, Some(original.clone())),
            Value::Bool(false)
        ]
    );
    assert_eq!(other_base, [Value::Variant(2, Some(original))]);
    assert_eq!(Arc::strong_count(&payload), payload_refs);
    assert_eq!(Arc::strong_count(shared), list_refs);
    replay.count
}
