//! Ranked host visits for a mixed record, list, and variant DAG.
use super::{CELLS, FRAMES, FUEL, Replay, list_effect_checks::list};
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};
use std::{collections::HashSet, sync::Arc};

const SOURCE: &str = include_str!("shared_host_admission.fluxr");
const EXPORT: &str = "admit";
const LEVELS: usize = 9;
const LEAF_CELLS: usize = 7;
const BAD_LEAF: &str = "host value differs from I64";
const BAD_FLAG: &str = "host value differs from Bool";
const BAD_TAG: &str = "unknown host enum tag";
const BAD_PAYLOAD: &str = "host enum payload differs from interface";
const COUNT: &str = "host argument or input count differs from interface";
type Admission = Result<usize, (usize, &'static str)>;
type Outcome = Result<Value, &'static str>;

fn leaf(second: Value, flag: Value) -> Value {
    Value::Record([list([list([list([Value::I64(5), second])])]), flag].into())
}

fn layer(child: Value, width: usize, last: Option<Value>) -> Value {
    let more = Value::Variant(1, Some(Arc::new(child)));
    let children = std::iter::once(Value::Variant(0, None))
        .chain(std::iter::repeat_n(more.clone(), width - 1))
        .chain([last.unwrap_or(more)]);
    Value::Record([list(children), Value::Bool(true)].into())
}

fn tree(leaf: &Value, width: usize, last: Option<Value>) -> Value {
    let child = (1..LEVELS).fold(leaf.clone(), |child, _| layer(child, width, None));
    layer(child, width, last)
}

fn expand(value: &Value) -> Value {
    match value {
        Value::Record(fields) => Value::Record(fields.iter().map(expand).collect()),
        Value::List(values) => list(values.iter().map(expand)),
        Value::Variant(tag, Some(payload)) => Value::Variant(*tag, Some(Arc::new(expand(payload)))),
        other => other.clone(),
    }
}

fn physical_allocations(roots: &[Value]) -> usize {
    fn visit(value: &Value, seen: &mut HashSet<usize>) {
        match value {
            Value::Record(fields) => {
                if seen.insert(fields.as_ptr() as usize) {
                    fields.iter().for_each(|field| visit(field, seen));
                }
            }
            Value::List(values) => {
                if seen.insert(Arc::as_ptr(values) as usize) {
                    values.iter().for_each(|item| visit(item, seen));
                }
            }
            Value::Variant(_, Some(payload)) => {
                if seen.insert(Arc::as_ptr(payload) as usize) {
                    visit(payload, seen);
                }
            }
            _ => {}
        }
    }

    let mut seen = HashSet::new();
    roots.iter().for_each(|root| visit(root, &mut seen));
    seen.len()
}

// One layer adds a record, list, empty tag, trailing flag, and one tag
// per child. The oracle uses ranks and counts without walking host values.
fn visits(levels: usize, width: usize) -> usize {
    (0..levels).fold(LEAF_CELLS, |n, _| 4 + width * (1 + n))
}

fn last_tag_rank(width: usize) -> usize {
    4 + (width - 1) * (1 + visits(LEVELS - 1, width))
}

fn outcome(roots: [Admission; 4], limits: Limits) -> Outcome {
    let mut prior = 0;
    for root in roots {
        match root {
            Ok(count) if prior + count <= limits.cells => prior += count,
            Ok(_) => return Err(CELLS),
            Err((rank, error)) => {
                return Err(if prior + rank <= limits.cells {
                    error
                } else {
                    CELLS
                });
            }
        }
    }
    if limits.frames == 0 {
        return Err(FRAMES);
    }
    if limits.cells < prior + 3 {
        return Err(CELLS); // Frame and two argument slots.
    }
    if limits.instructions < 2 {
        return Err(FUEL); // Literal and return.
    }
    if limits.cells < prior + 4 {
        return Err(CELLS); // Scalar result.
    }
    Ok(Value::U64(7))
}

fn call<'a>(inputs: &'a [Value], arguments: &'a [Value], limits: Limits) -> Invocation<'a> {
    Invocation {
        export: EXPORT,
        inputs,
        arguments,
        limits,
    }
}

struct Probes<'a> {
    replay: Replay<'a>,
    source: ModuleSlot,
    workspace: Workspace,
}

impl Probes<'_> {
    fn check(
        &mut self,
        label: &str,
        inputs: &[Value],
        args: &[Value],
        limits: Limits,
        want: Outcome,
    ) {
        self.replay
            .check_routes_labeled(inputs, args, limits, want.clone(), label, |_| {});
        let actual = self
            .source
            .run_with(call(inputs, args, limits), &mut self.workspace)
            .map_err(|error| error.to_string());
        assert_eq!(
            actual.as_ref().map_err(String::as_str),
            want.as_ref().map_err(|&error| error),
            "source injection {label}, {limits:?}"
        );
    }
}

fn check_replacement(
    mut slot: ModuleSlot,
    good: &[Value; 4],
    bad: &[Value; 4],
    admit_cells: usize,
) {
    let original = slot.module().encode();
    let replacement = Module::compile_compact(&SOURCE.replace("(ret 7u)", "(ret 8u)")).unwrap();
    let bytes = replacement.encode();
    assert_ne!(bytes, original);
    let full = Limits {
        instructions: 2,
        cells: admit_cells + 4,
        frames: 1,
    };
    for (cells, trap) in [(41, CELLS), (42, BAD_LEAF)] {
        let limits = Limits { cells, ..full };
        let error = slot
            .inject(
                1,
                replacement.clone(),
                Some(call(&bad[..2], &bad[2..], limits)),
            )
            .unwrap_err();
        assert_eq!(error.to_string(), trap);
        assert_eq!(slot.revision(), 1);
        assert_eq!(slot.module().encode(), original);
        assert_eq!(
            slot.run(call(&good[..2], &good[2..], full)).unwrap(),
            Value::U64(7)
        );
    }
    assert_eq!(
        slot.inject(1, replacement, Some(call(&good[..2], &good[2..], full)))
            .unwrap(),
        2
    );
    assert_eq!(slot.module().encode(), bytes);
    assert_eq!(
        slot.run(call(&good[..2], &good[2..], full)).unwrap(),
        Value::U64(8)
    );
}

pub(crate) fn check_shared_host_admission() -> usize {
    // Deepest scalar is at depth 9 * 3 + 4 = 31, with a zero-based root.
    let too_deep = SOURCE.replace("leaf:[[[i64]]]", "leaf:[[[[i64]]]]");
    assert_eq!(
        Module::compile_compact(&too_deep)
            .err()
            .unwrap()
            .to_string(),
        "type nesting limit exceeded"
    );
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native(EXPORT));
    let grants = [("tree", "R9"), ("leaf", "R0")].map(|(name, ty)| Binding {
        name: name.into(),
        ty: Type::Record(ty.into()),
    });
    let mut source = ModuleSlot::new(module.clone(), &grants).unwrap();
    assert_eq!(
        source
            .inject(0, Module::compile_compact(SOURCE).unwrap(), None)
            .unwrap(),
        1
    );
    let mut probes = Probes {
        replay: Replay::new(module, EXPORT, &[], &grants),
        source,
        workspace: Workspace::default(),
    };

    let base = leaf(Value::I64(9), Value::Bool(true));
    // The bad leaf carries an extra level that must never be descended into.
    let bad_leaf = leaf(list([Value::I64(9)]), Value::Bool(true));
    let bad_flag = leaf(Value::I64(9), Value::Unit);
    let good_tree = tree(&base, 2, None);
    let bad_tree = tree(&bad_leaf, 2, None);
    let bad_tag = tree(&base, 2, Some(Value::Variant(2, None)));
    let missing = tree(&base, 2, Some(Value::Variant(1, None)));
    let good = [
        good_tree.clone(),
        base.clone(),
        good_tree.clone(),
        base.clone(),
    ];
    let deep_rank = 4 * LEVELS + 6;
    let tree_cells = visits(LEVELS, 2);
    let admit_cells = 2 * (tree_cells + LEAF_CELLS);
    assert_eq!((deep_rank, tree_cells, admit_cells), (42, 6650, 13314));
    assert_eq!(physical_allocations(&good), 31);
    assert_eq!(physical_allocations(&good.each_ref().map(expand)), 8192);

    let tree_ok = (&good_tree, Ok(tree_cells));
    let leaf_ok = (&base, Ok(LEAF_CELLS));
    let deep_bad = (&bad_tree, Err((deep_rank, BAD_LEAF)));
    let shallow_bad = (&bad_leaf, Err((6, BAD_LEAF)));
    let flag_bad = (&bad_flag, Err((7, BAD_FLAG)));
    let tag_bad = (&bad_tag, Err((last_tag_rank(2), BAD_TAG)));
    let payload_bad = (&missing, Err((last_tag_rank(2), BAD_PAYLOAD)));
    for (label, roots, first_bad) in [
        ("valid", [tree_ok, leaf_ok, tree_ok, leaf_ok], None),
        (
            "first input",
            [deep_bad, shallow_bad, tag_bad, flag_bad],
            Some(deep_rank),
        ),
        (
            "second input",
            [tree_ok, flag_bad, tag_bad, shallow_bad],
            Some(tree_cells + 7),
        ),
        (
            "first argument",
            [tree_ok, leaf_ok, tag_bad, shallow_bad],
            Some(tree_cells + LEAF_CELLS + last_tag_rank(2)),
        ),
        (
            "first argument payload",
            [tree_ok, leaf_ok, payload_bad, flag_bad],
            Some(tree_cells + LEAF_CELLS + last_tag_rank(2)),
        ),
        (
            "second argument",
            [tree_ok, leaf_ok, tree_ok, flag_bad],
            Some(admit_cells),
        ),
    ] {
        let shared = roots.map(|(value, _)| value.clone());
        let expanded = shared.each_ref().map(expand);
        assert!(shared == expanded, "{label} expansion changed the value");
        let valid_caps = [
            0,
            1,
            deep_rank - 1,
            deep_rank,
            tree_cells - 1,
            tree_cells,
            tree_cells + LEAF_CELLS,
            admit_cells - 1,
            admit_cells,
            admit_cells + 1,
            admit_cells + 2,
            admit_cells + 3,
            admit_cells + 4,
            admit_cells + 5,
        ];
        let cells = first_bad.map(|rank| [0, rank - 1, rank, rank + 1]);
        let caps = cells.as_ref().map_or(&valid_caps[..], |caps| &caps[..]);
        for values in [&shared, &expanded] {
            for &cells in caps {
                for instructions in 0..=2 {
                    for frames in 0..=1 {
                        let limits = Limits {
                            instructions,
                            cells,
                            frames,
                        };
                        probes.check(
                            label,
                            &values[..2],
                            &values[2..],
                            limits,
                            outcome(roots.map(|(_, admission)| admission), limits),
                        );
                    }
                }
            }
        }
    }

    let blocked = Limits {
        instructions: 0,
        cells: 0,
        frames: 0,
    };
    // Count wins even when the first supplied value is malformed.
    for values in [
        [
            bad_tree.clone(),
            bad_leaf.clone(),
            missing.clone(),
            bad_flag.clone(),
        ],
        [
            expand(&bad_tree),
            expand(&bad_leaf),
            expand(&missing),
            expand(&bad_flag),
        ],
    ] {
        probes.check(
            "input count",
            &values[..1],
            &values[2..],
            blocked,
            Err(COUNT),
        );
        probes.check(
            "argument count",
            &values[..2],
            &values[2..3],
            blocked,
            Err(COUNT),
        );
    }

    // 31 allocations represent over a billion visits in a single large root.
    // Keep this fixture shared and every probe capped, including the probe
    // with a malformed tag after seven logical subtrees.
    let large = tree(&base, 8, None);
    let early = tree(&bad_leaf, 8, None);
    let late = tree(&base, 8, Some(Value::Variant(2, None)));
    let large_cells = visits(LEVELS, 8);
    assert_eq!(large_cells, 1_169_611_628);
    assert_eq!(physical_allocations(std::slice::from_ref(&large)), 31);
    assert!(last_tag_rank(8) > 4096);
    for (label, value, admission) in [
        ("large valid DAG", large, Ok(large_cells)),
        ("large early bad leaf", early, deep_bad.1),
        ("large late bad tag", late, Err((last_tag_rank(8), BAD_TAG))),
    ] {
        for cells in [0, 31, 41, 42, 43, 256, 4096] {
            let limits = Limits { cells, ..blocked };
            probes.check(
                label,
                &[value.clone(), base.clone()],
                &good[2..],
                limits,
                outcome([admission, leaf_ok.1, tree_ok.1, leaf_ok.1], limits),
            );
        }
    }
    let bad = [bad_tree, bad_leaf, missing, bad_flag];
    check_replacement(probes.source, &good, &bad, admit_cells);
    probes.replay.count
}
