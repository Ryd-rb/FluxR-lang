//! Bounded replay of the accepted I/J/K effect ledgers through canonical FXH.
use super::super::{CELLS, FRAMES, FUEL, list_boundary_checks, list_effect_checks::list};
use super::super::{shared_host_admission_checks as shared, variant_payload_checks};
use super::{alias, restored};
use fluxr::hir::Hir;
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use std::sync::Arc;

type Outcome = Result<Value, &'static str>;

fn limits((instructions, cells, frames): (u64, usize, usize)) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

fn signed_list(values: &[i64]) -> Value {
    list(values.iter().copied().map(Value::I64))
}

fn check(
    module: &Module,
    workspace: &mut Workspace,
    (id, export): (&str, &str),
    (inputs, arguments): (&[Value], &[Value]),
    limits: Limits,
    expected: Outcome,
) -> Option<Value> {
    let actual = module
        .run_portable_with(
            Invocation {
                export,
                inputs,
                arguments,
                limits,
            },
            workspace,
        )
        .map_err(|err| err.to_string());
    assert_eq!(
        actual.as_ref().map_err(String::as_str),
        expected.as_ref().map_err(|&err| err),
        "{id} {limits:?}"
    );
    actual.ok()
}

fn check_list(module: &Module, workspace: &mut Workspace) -> usize {
    const HIGH: u64 = 1 << 32;
    const BOUNDS: &str = "list index out of bounds";
    let row = signed_list(&[3, 5]);
    let rows = list([row.clone(), row.clone()]);
    let short = list([row.clone()]);
    let good = signed_list(&[3, 5, 5]);
    #[rustfmt::skip]
    let cases = [
        ("list-exact", &rows, 0, 1, (18, 32, 2), Ok(good)),
        ("list-fuel-short", &rows, 0, 1, (17, 32, 2), Err(FUEL)),
        ("list-cells-short", &rows, 0, 1, (18, 31, 2), Err(CELLS)),
        ("list-frames-short", &rows, 0, 1, (18, 32, 1), Err(FRAMES)),
        ("list-row", &rows, 2, 1, (3, 20, 1), Err(BOUNDS)),
        ("list-row-high", &rows, HIGH, 1, (3, 20, 1), Err(BOUNDS)),
        ("list-row-before-fuel", &rows, HIGH, 1, (2, 20, 1), Err(FUEL)),
        ("list-snapshot-row-after-write", &short, 1, 1, (12, 24, 2), Err(BOUNDS)),
        ("list-column-after-write", &rows, 0, 2, (14, 27, 2), Err(BOUNDS)),
        ("list-column-high", &rows, 0, HIGH | 1, (14, 27, 2), Err(BOUNDS)),
        ("list-column-fuel-short", &rows, 0, HIGH | 1, (13, 27, 2), Err(FUEL)),
        ("list-column-copy-short", &rows, 0, HIGH | 1, (14, 26, 2), Err(CELLS)),
    ];
    let count = cases.len();
    for (id, base, i, j, cap, expected) in cases {
        let inputs = [base.clone()];
        let args = [rows.clone(), Value::U64(i), Value::U64(j)];
        let cap = limits(cap);
        assert_eq!(
            list_boundary_checks::ledger(&inputs, &args, cap, false),
            expected,
            "{id}"
        );
        let actual = check(
            module,
            workspace,
            (id, "edit"),
            (&inputs, &args),
            cap,
            expected,
        );
        if let Some(value) = actual {
            assert!(!alias(&value, &row), "{id}");
        }
        assert!(alias(&inputs[0], base) && alias(&args[0], &rows), "{id}");
        for root in [&inputs[0], &args[0]] {
            let Value::List(children) = root else {
                unreachable!()
            };
            assert!(children.iter().all(|child| alias(child, &row)), "{id}");
        }
        assert_eq!(row, signed_list(&[3, 5]), "{id}");
    }
    count
}

fn check_variant(module: &Module, workspace: &mut Workspace) -> usize {
    const WRONG: &str = "enum payload does not match variant";
    const MISSING: &str = "host enum payload differs from interface";
    let row = signed_list(&[3, 5]);
    let payload = Arc::new(row.clone());
    let edit = Value::Variant(1, Some(payload.clone()));
    let other = Value::Variant(2, Some(payload));
    let empty = Value::Variant(0, None);
    let missing = Value::Variant(1, None);
    #[rustfmt::skip]
    let cases = [
        ("variant-exact", &edit, &edit, false, (18, 24, 2), Ok(signed_list(&[3, 5, 7, 3]))),
        ("variant-fuel-short", &edit, &edit, false, (17, 24, 2), Err(FUEL)),
        ("variant-cells-short", &edit, &edit, false, (18, 23, 2), Err(CELLS)),
        ("variant-frames-short", &edit, &edit, false, (18, 24, 1), Err(FRAMES)),
        ("variant-guard-other", &edit, &other, false, (7, 14, 1), Ok(list([]))),
        ("variant-guard-empty", &edit, &empty, false, (7, 11, 1), Ok(list([]))),
        ("variant-force-other", &edit, &other, true, (9, 15, 2), Err(WRONG)),
        ("variant-force-empty", &edit, &empty, true, (9, 12, 2), Err(WRONG)),
        ("variant-snapshot-after-write", &other, &edit, false, (14, 18, 2), Err(WRONG)),
        ("variant-copy-short", &other, &edit, false, (14, 17, 2), Err(CELLS)),
        ("variant-payload-missing", &edit, &missing, false, (0, 5, 0), Err(MISSING)),
        ("variant-admission-short", &edit, &missing, false, (0, 4, 0), Err(CELLS)),
    ];
    let count = cases.len();
    for (id, base, selected, force, cap, expected) in cases {
        let inputs = [base.clone()];
        let args = [selected.clone(), Value::Bool(force)];
        let cap = limits(cap);
        assert_eq!(
            variant_payload_checks::ledger(&inputs, &args, cap, 1),
            expected,
            "{id}"
        );
        let actual = check(
            module,
            workspace,
            (id, "edit"),
            (&inputs, &args),
            cap,
            expected,
        );
        if let Some(value) = actual {
            assert!(!alias(&value, &row), "{id}");
        }
        for (value, original) in [(&inputs[0], base), (&args[0], selected)] {
            if let Value::Variant(_, Some(child)) = value {
                assert!(alias(value, original) && alias(child, &row), "{id}");
            } else {
                assert_eq!(value, original, "{id}");
            }
        }
        assert_eq!(row, signed_list(&[3, 5]), "{id}");
    }
    count
}

fn check_shared(module: &Module, workspace: &mut Workspace) -> usize {
    const BAD_LEAF: &str = "host value differs from I64";
    let leaf = shared::leaf(Value::I64(9), Value::Bool(true));
    let tree = shared::tree(&leaf, 2, None);
    let bad_leaf = shared::leaf(list([Value::I64(9)]), Value::Bool(true));
    let bad_tree = shared::tree(&bad_leaf, 2, None);
    let roots = [tree.clone(), leaf.clone(), tree.clone(), leaf.clone()];
    let expanded = roots.each_ref().map(shared::expand);
    assert_eq!(roots, expanded);
    let deep = [bad_tree, leaf.clone(), tree.clone(), leaf.clone()];

    let tree_cells = shared::visits(9, 2);
    let admission = [Ok(tree_cells), Ok(7), Ok(tree_cells), Ok(7)];
    let mut bad_admission = admission;
    bad_admission[0] = Err((42, BAD_LEAF));
    #[rustfmt::skip]
    let cases = [
        ("shared-exact", &roots, admission, (2, 13_318, 1), Ok(Value::U64(7))),
        ("shared-fuel-short", &roots, admission, (1, 13_318, 1), Err(FUEL)),
        ("shared-result-cell-short", &roots, admission, (2, 13_317, 1), Err(CELLS)),
        ("shared-frame-short", &roots, admission, (2, 13_318, 0), Err(FRAMES)),
        ("expanded-exact", &expanded, admission, (2, 13_318, 1), Ok(Value::U64(7))),
        ("expanded-result-cell-short", &expanded, admission, (2, 13_317, 1), Err(CELLS)),
        ("shared-deep-bad-leaf", &deep, bad_admission, (0, 42, 0), Err(BAD_LEAF)),
        ("shared-deep-rank-short", &deep, bad_admission, (0, 41, 0), Err(CELLS)),
    ];
    let count = cases.len();
    for (id, values, charges, cap, expected) in cases {
        let cap = limits(cap);
        assert_eq!(shared::outcome(charges, cap), expected, "{id}");
        check(
            module,
            workspace,
            (id, "admit"),
            (&values[..2], &values[2..]),
            cap,
            expected,
        );
        assert!(
            alias(&roots[0], &roots[2]) && alias(&roots[1], &roots[3]),
            "{id}"
        );
        assert_eq!(shared::physical_allocations(&roots), 31, "{id}");
        assert_eq!(roots, expanded, "{id}");
    }
    count
}

pub(crate) fn check_nested_payload_hir_replay() -> usize {
    let fixtures = [
        (
            fixture!("list_boundary"),
            check_list as fn(&Module, &mut Workspace) -> usize,
        ),
        (fixture!("variant_payload"), check_variant),
        (fixture!("shared_host_admission"), check_shared),
    ];
    let mut checked = 0;
    for ((name, source, fsm, fxh), check_calls) in fixtures {
        let decoded = Module::decode(fsm).unwrap();
        for module in [Module::compile_compact(source).unwrap(), decoded.clone()] {
            assert_eq!(module.encode(), fsm, "{name}");
            assert_eq!(
                Hir::from_structured(module).encode().unwrap(),
                fxh,
                "{name}"
            );
        }
        let hir = Hir::decode(fxh).unwrap();
        assert_eq!(hir.encode().unwrap(), fxh, "{name}");
        let recompiled = Module::compile(hir.source().unwrap()).unwrap();
        for module in [decoded, recompiled, restored(hir)] {
            assert_eq!(module.encode(), fsm, "{name}");
            checked += check_calls(&module, &mut Workspace::default());
        }
    }
    checked
}
