use fluxr::collaboration::{
    Change, DeclarationKey, DependencyManifest, Edit, Mode, Snapshot, Target,
};
use fluxr::structured::{Expr, Source, Stmt};
use std::collections::BTreeSet;

fn target(name: &str) -> Target {
    Target {
        id: name.into(),
        function: name.into(),
        receiver: None,
        mode: Mode::Exclusive,
    }
}

fn source() -> Source {
    Source::parse_compact(
        "v1 (fn leaf () i64 () (ret 1))
         (fn helper () i64 () (ret (call leaf)))
         (pub caller () i64 () (ret (call helper)))
         (pub unrelated () i64 () (ret 0))",
    )
    .unwrap()
}

fn snapshot(source: Source) -> Snapshot {
    Snapshot::new(source, ["leaf", "caller", "unrelated"].map(target).to_vec()).unwrap()
}

fn key(name: &str) -> DeclarationKey {
    DeclarationKey {
        receiver: None,
        name: name.into(),
    }
}

fn edit(base: &Snapshot, name: &str, value: Expr) -> Change {
    Change {
        author: "query-tests".into(),
        requires: Default::default(),
        edits: vec![Edit::ReplaceBody {
            target: name.into(),
            expected: base.edit_revision(name).unwrap(),
            body: vec![Stmt::Return { value }],
        }],
    }
}

#[test]
fn manifests_include_unregistered_helpers_with_stable_order_and_round_trip() {
    let base = snapshot(source());
    let manifest = base.dependency_manifest().unwrap();
    let helper = manifest
        .declarations
        .iter()
        .find(|entry| entry.declaration == key("helper"))
        .unwrap();
    assert_eq!(helper.target, None);
    assert_eq!(helper.callees, vec![key("leaf")]);
    let caller = manifest
        .declarations
        .iter()
        .find(|entry| entry.declaration == key("caller"))
        .unwrap();
    assert_eq!(caller.callees, vec![key("helper")]);
    assert_eq!(caller.revision, base.revision("caller").unwrap());
    let mut reordered = source();
    reordered.functions.reverse();
    let reordered = snapshot(reordered);
    let reversed = reordered.dependency_manifest().unwrap();
    assert_eq!(manifest.declarations, reversed.declarations);
    assert_eq!(manifest.environment, reversed.environment);
    assert!(
        reordered
            .impact_since(&base)
            .unwrap()
            .affected_targets
            .is_empty()
    );
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        serde_json::from_slice::<DependencyManifest>(&encoded).unwrap(),
        manifest
    );
}

#[test]
fn reverse_impact_includes_transitive_callers_and_rejects_unknown_targets() {
    let base = snapshot(source());
    assert_eq!(
        base.affected_targets(["leaf"]).unwrap(),
        BTreeSet::from(["caller".into(), "leaf".into()])
    );
    assert_eq!(
        base.affected_targets(["unrelated", "unrelated"]).unwrap(),
        BTreeSet::from(["unrelated".into()])
    );
    assert!(base.affected_targets(["missing"]).is_err());
    assert!(base.affected_targets([]).unwrap().is_empty());
}

#[test]
fn comparison_finds_unregistered_helper_changes_and_schema_changes() {
    let base = snapshot(source());
    let mut altered = source();
    altered.functions[1].body = vec![Stmt::Return {
        value: Expr::I64 { value: 4 },
    }];
    let next = snapshot(altered);
    let impact = next.impact_since(&base).unwrap();
    assert_eq!(impact.changed_declarations, vec![key("helper")]);
    assert_eq!(impact.affected_targets, vec!["caller"]);
    assert!(!impact.environment_changed);
    let schema = Snapshot::new(
        Source::parse_compact(
            "v1 (rec Context value:i64)
             (fn leaf () i64 () (ret 1))
             (fn helper () i64 () (ret (call leaf)))
             (pub caller () i64 () (ret (call helper)))
             (pub unrelated () i64 () (ret 0))",
        )
        .unwrap(),
        ["leaf", "caller", "unrelated"].map(target).to_vec(),
    )
    .unwrap();
    let impact = schema.impact_since(&base).unwrap();
    assert!(impact.environment_changed);
    assert_eq!(impact.affected_targets, vec!["caller", "leaf", "unrelated"]);
}

#[test]
fn cache_invalidation_tracks_edge_removal_then_addition() {
    let base = snapshot(source());
    let base_caller = base.behavior_revision("caller").unwrap();
    let removed = base
        .prepare(&[edit(&base, "caller", Expr::I64 { value: 7 })])
        .unwrap();
    let disconnected = removed.snapshot();
    assert_ne!(
        disconnected.behavior_revision("caller").unwrap(),
        base_caller
    );
    let disconnected_caller = disconnected.behavior_revision("caller").unwrap();
    let leaf_edit = disconnected
        .prepare(&[edit(disconnected, "leaf", Expr::I64 { value: 8 })])
        .unwrap();
    let after_leaf = leaf_edit.snapshot();
    assert_eq!(
        after_leaf.behavior_revision("caller").unwrap(),
        disconnected_caller
    );
    let connected = after_leaf
        .prepare(&[edit(
            after_leaf,
            "caller",
            Expr::Call {
                name: "leaf".into(),
                args: vec![],
            },
        )])
        .unwrap();
    let connected = connected.snapshot();
    let connected_caller = connected.behavior_revision("caller").unwrap();
    let next_leaf = connected
        .prepare(&[edit(connected, "leaf", Expr::I64 { value: 9 })])
        .unwrap();
    assert_ne!(
        next_leaf.snapshot().behavior_revision("caller").unwrap(),
        connected_caller
    );
    // Rebuilding from the emitted admitted FSM must reproduce the warm result.
    let body = next_leaf.snapshot().module().encode();
    let source_json = std::str::from_utf8(&body[6..]).unwrap();
    // The public compact round-trip exposes exactly the same admitted module.
    let cold_module = fluxr::structured::Module::compile_legacy_json(source_json).unwrap();
    assert_eq!(cold_module.encode(), body);
}

#[test]
fn recursive_graphs_and_parallel_snapshot_readers_are_isolated() {
    let source = Source::parse_compact(
        "v1 (fn even (n:i64) bool () (if (== n 0) ((ret true)) ((ret (call odd (- n 1))))))
         (fn odd (n:i64) bool () (if (== n 0) ((ret false)) ((ret (call even (- n 1))))))
         (pub query (n:i64) bool () (ret (call even n)))",
    )
    .unwrap();
    let base = Snapshot::new(source, ["even", "odd", "query"].map(target).to_vec()).unwrap();
    assert_eq!(
        base.affected_targets(["odd"]).unwrap(),
        BTreeSet::from(["even".into(), "odd".into(), "query".into()])
    );
    let candidate = base
        .prepare(&[edit(&base, "odd", Expr::Bool { value: false })])
        .unwrap();
    let next = candidate.snapshot();
    let before = base.behavior_revision("query").unwrap();
    let after = next.behavior_revision("query").unwrap();
    assert_ne!(before, after);
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                for _ in 0..20 {
                    assert_eq!(base.behavior_revision("query").unwrap(), before);
                    assert_eq!(next.behavior_revision("query").unwrap(), after);
                }
            });
        }
    });
}

#[test]
fn added_removed_and_remapped_targets_are_reported() {
    let base = snapshot(source());
    let mut remapped = target("leaf");
    remapped.function = "unrelated".into();
    let next = Snapshot::new(source(), vec![target("caller"), target("helper"), remapped]).unwrap();
    let impact = next.impact_since(&base).unwrap();
    assert_eq!(impact.added_targets, vec!["helper"]);
    assert_eq!(impact.removed_targets, vec!["unrelated"]);
    assert_eq!(impact.affected_targets, vec!["helper", "leaf"]);
    assert!(impact.changed_declarations.is_empty());
}

#[test]
fn mode_only_changes_are_reported_as_contract_and_check_impact() {
    let source = Source::parse_compact("v1 (pub limit () f32 () (ret 1f))").unwrap();
    let make = |mode| {
        Snapshot::new(
            source.clone(),
            vec![Target {
                id: "limit".into(),
                function: "limit".into(),
                receiver: None,
                mode,
            }],
        )
        .unwrap()
    };
    let before = make(Mode::Exclusive);
    let after = make(Mode::MinF32);
    assert_eq!(
        before.behavior_revision("limit").unwrap(),
        after.behavior_revision("limit").unwrap()
    );
    let impact = after.impact_since(&before).unwrap();
    assert!(!impact.environment_changed);
    assert!(impact.changed_declarations.is_empty());
    assert_eq!(impact.changed_contracts, vec!["limit"]);
    assert_eq!(impact.affected_targets, vec!["limit"]);
}

#[test]
fn warm_queries_match_cold_compilation_through_a_generated_edit_sequence() {
    let mut cold_source = source();
    let mut warm = snapshot(cold_source.clone());
    let mut preserved = Vec::new();
    for step in 0..72 {
        let (name, index, value) = match step % 3 {
            0 => ("leaf", 0, Expr::I64 { value: step }),
            1 if step % 2 == 0 => (
                "caller",
                2,
                Expr::Call {
                    name: "helper".into(),
                    args: vec![],
                },
            ),
            1 => ("caller", 2, Expr::I64 { value: step }),
            _ => ("unrelated", 3, Expr::I64 { value: step }),
        };
        let previous_pin = warm.behavior_revision("caller").unwrap();
        preserved.push((warm.clone(), previous_pin));
        let candidate = warm.prepare(&[edit(&warm, name, value.clone())]).unwrap();
        warm = candidate.snapshot().clone();
        cold_source.functions[index].body = vec![Stmt::Return { value }];
        let cold = snapshot(cold_source.clone());
        for target in ["leaf", "caller", "unrelated"] {
            assert_eq!(
                warm.behavior_revision(target).unwrap(),
                cold.behavior_revision(target).unwrap()
            );
        }
        assert_eq!(warm.module().encode(), cold.module().encode());
    }
    for (old, pin) in preserved {
        assert_eq!(old.behavior_revision("caller").unwrap(), pin);
    }
}

#[test]
fn graph_impact_matches_exhaustive_pins_through_recursion_and_edge_replacement() {
    // Build a chain with a back edge; every fourth declaration is an unregistered
    // helper. Repeated edits connect/disconnect components without changing the
    // stable target contract, and compare against full closure hashes.
    let count = 24;
    let names: Vec<_> = (0..count).map(|i| format!("g{i:02}")).collect();
    let mut text = String::from("v1 ");
    for (i, name) in names.iter().enumerate() {
        let callee = if i + 1 == count { 8 } else { i + 1 };
        text.push_str(&format!(
            "(fn {name} () i64 () (ret (call {}))) ",
            names[callee]
        ));
    }
    text.push_str("(pub output () i64 () (ret 0))");
    let targets: Vec<_> = names
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 4 != 3)
        .map(|(_, name)| target(name))
        .chain(std::iter::once(target("output")))
        .collect();
    let tracked: Vec<_> = targets.iter().map(|t| t.id.clone()).collect();
    let mut current = Snapshot::new(Source::parse_compact(&text).unwrap(), targets).unwrap();
    let mut history = vec![current.clone()];
    for step in 0..90 {
        let name = &tracked[step % tracked.len()];
        let value = if step % 3 == 0 {
            Expr::I64 { value: step as i64 }
        } else {
            Expr::Call {
                name: names[(step * 7 + 3) % count].clone(),
                args: vec![],
            }
        };
        current = current
            .prepare(&[edit(&current, name, value)])
            .unwrap()
            .snapshot()
            .clone();
        for before in [history.last().unwrap(), &history[step % history.len()]] {
            let expected: Vec<_> = tracked
                .iter()
                .filter(|target| {
                    current.behavior_revision(target).unwrap()
                        != before.behavior_revision(target).unwrap()
                })
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let impact = current.impact_since(before).unwrap();
            assert_eq!(impact.affected_targets, expected, "step {step}");
            assert!(impact.changed_contracts.is_empty());
            assert!(!impact.environment_changed);
            // A manifest must agree with independently hashed target revisions.
            for declaration in current.dependency_manifest().unwrap().declarations {
                if let Some(target) = declaration.target {
                    assert_eq!(Some(declaration.revision), current.revision(&target));
                }
            }
        }
        history.push(current.clone());
    }
}

#[test]
fn old_graph_accounts_for_deleted_helpers_and_new_graph_accounts_for_added_edges() {
    let before = Snapshot::new(
        Source::parse_compact(
            "v1 (fn old () i64 () (ret 1))
         (pub caller () i64 () (ret (call old)))
         (pub independent () i64 () (ret 7))",
        )
        .unwrap(),
        vec![target("caller"), target("independent")],
    )
    .unwrap();
    let after = Snapshot::new(
        Source::parse_compact(
            "v1 (pub independent () i64 () (ret 7))
         (fn new () i64 () (ret 2))
         (pub caller () i64 () (ret (call new)))",
        )
        .unwrap(),
        vec![target("caller"), target("independent")],
    )
    .unwrap();
    let impact = after.impact_since(&before).unwrap();
    assert_eq!(
        impact.changed_declarations,
        vec![key("caller"), key("new"), key("old")]
    );
    assert_eq!(impact.affected_targets, vec!["caller"]);
    assert_eq!(
        after.affected_targets(["independent"]).unwrap(),
        BTreeSet::from(["independent".into()])
    );
    assert_eq!(
        before.affected_targets(["caller"]).unwrap(),
        BTreeSet::from(["caller".into()])
    );
}

#[test]
fn impact_target_order_is_independent_of_declaration_slots_and_target_spellings() {
    let source = Source::parse_compact(
        "v1 (fn first () i64 () (ret 0)) (pub second () i64 () (ret (call first)))",
    )
    .unwrap();
    let base = Snapshot::new(
        source,
        vec![
            Target {
                id: "z-last".into(),
                function: "first".into(),
                receiver: None,
                mode: Mode::Exclusive,
            },
            Target {
                id: "a-first".into(),
                function: "second".into(),
                receiver: None,
                mode: Mode::Exclusive,
            },
        ],
    )
    .unwrap();
    let expected = vec!["a-first".to_owned(), "z-last".to_owned()];
    assert_eq!(
        base.affected_targets(["z-last"])
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        expected
    );
    let candidate = base
        .prepare(&[edit(&base, "z-last", Expr::I64 { value: 5 })])
        .unwrap();
    assert_eq!(
        candidate
            .snapshot()
            .impact_since(&base)
            .unwrap()
            .affected_targets,
        expected
    );
}
