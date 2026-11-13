use fluxr::collaboration::{Change, CheckedChange, Edit, Mode, Rejection, Snapshot, Target};
use fluxr::structured::{Expr, Source, Stmt};
use std::collections::BTreeMap;

fn target(name: &str) -> Target {
    Target {
        id: name.into(),
        function: name.into(),
        receiver: None,
        mode: Mode::Exclusive,
    }
}

fn graph() -> Snapshot {
    Snapshot::new(
        Source::parse_compact(
            "v1
         (fn leaf () i64 () (ret 1))
         (fn helper () i64 () (ret (call leaf)))
         (pub caller () i64 () (ret (call helper)))
         (pub output () i64 () (ret 0))
         (pub aux () i64 () (ret 0))",
        )
        .unwrap(),
        ["leaf", "caller", "output", "aux"].map(target).to_vec(),
    )
    .unwrap()
}

fn edit(snapshot: &Snapshot, name: &str, value: i64) -> Edit {
    Edit::ReplaceBody {
        target: name.into(),
        expected: snapshot.edit_revision(name).unwrap(),
        body: vec![Stmt::Return {
            value: Expr::I64 { value },
        }],
    }
}

fn change(snapshot: &Snapshot, name: &str, value: i64) -> Change {
    Change {
        author: name.into(),
        requires: BTreeMap::new(),
        edits: vec![edit(snapshot, name, value)],
    }
}

fn checked(snapshot: &Snapshot, name: &str, value: i64, reads: &[&str]) -> CheckedChange {
    CheckedChange {
        change: change(snapshot, name, value),
        behavior_requires: snapshot.behavior_read_set(reads.iter().copied()).unwrap(),
    }
}

#[test]
fn indirect_helper_edits_change_behavior_but_preserve_legacy_declaration_pins() {
    let base = graph();
    let proposal = base.prepare(&[change(&base, "leaf", 2)]).unwrap();
    let next = proposal.snapshot();
    assert_eq!(base.revision("caller"), next.revision("caller"));
    assert_ne!(
        base.behavior_revision("caller").unwrap(),
        next.behavior_revision("caller").unwrap()
    );
    assert_eq!(
        base.behavior_revision("aux").unwrap(),
        next.behavior_revision("aux").unwrap()
    );

    let reader = checked(&base, "output", 1, &["caller"]);
    let result = next.prepare_checked(&[reader]).unwrap();
    assert!(result.accepted.is_empty());
    assert_eq!(result.snapshot().id(), next.id());
    assert!(
        matches!(result.rejected.values().next(), Some(Rejection::Stale(name)) if name == "caller")
    );
}

#[test]
fn same_batch_transitive_conflicts_reject_the_whole_reader_change_in_any_order() {
    let base = graph();
    let mut reader = checked(&base, "output", 1, &["caller"]);
    reader.change.edits.push(edit(&base, "aux", 3));
    let writer = checked(&base, "leaf", 2, &[]);
    let forward = base
        .prepare_checked(&[reader.clone(), writer.clone()])
        .unwrap();
    let backward = base.prepare_checked(&[writer, reader]).unwrap();
    assert_eq!(forward.accepted, backward.accepted);
    assert_eq!(forward.rejected, backward.rejected);
    assert_eq!(forward.snapshot().id(), backward.snapshot().id());
    assert_eq!(forward.accepted.len(), 1);
    assert_eq!(forward.snapshot().revision("aux"), base.revision("aux"));
    assert_eq!(
        forward.snapshot().revision("output"),
        base.revision("output")
    );
    assert!(
        matches!(forward.rejected.values().next(), Some(Rejection::DependencyChanged(name)) if name == "leaf")
    );
}

#[test]
fn unrelated_changes_and_identical_retries_are_independent() {
    let base = graph();
    let reader = checked(&base, "output", 1, &["caller"]);
    let unrelated = checked(&base, "aux", 4, &[]);
    let batch = base
        .prepare_checked(&[reader.clone(), unrelated.clone(), reader.clone()])
        .unwrap();
    assert_eq!(batch.accepted.len(), 2);
    assert!(batch.rejected.is_empty());
    let next = base.prepare_checked(&[unrelated]).unwrap();
    let late = next.snapshot().prepare_checked(&[reader]).unwrap();
    assert!(late.rejected.is_empty());
    assert_eq!(late.snapshot().id(), batch.snapshot().id());
}

#[test]
fn recursive_closures_terminate_and_ignore_function_traversal_order() {
    let mut source = Source::parse_compact(
        "v1
         (fn even (n:i64) bool () (if (== n 0) ((ret true)) ((ret (call odd (- n 1))))))
         (fn odd (n:i64) bool () (if (== n 0) ((ret false)) ((ret (call even (- n 1))))))
         (pub query (n:i64) bool () (ret (call even n)))
         (pub direct (n:i64) i64 () (if (== n 0) ((ret 0)) ((ret (call direct (- n 1))))))",
    )
    .unwrap();
    let make = |source| Snapshot::new(source, vec![target("query"), target("direct")]).unwrap();
    let first = make(source.clone());
    source.functions.reverse();
    let reversed = make(source.clone());
    for name in ["query", "direct"] {
        assert_eq!(
            first.behavior_revision(name).unwrap(),
            reversed.behavior_revision(name).unwrap()
        );
    }
    let odd = source
        .functions
        .iter_mut()
        .find(|function| function.name == "odd")
        .unwrap();
    odd.body = vec![Stmt::Return {
        value: Expr::Bool { value: false },
    }];
    let changed_unregistered = make(source);
    assert_ne!(
        first.behavior_revision("query").unwrap(),
        changed_unregistered.behavior_revision("query").unwrap()
    );
    assert_eq!(
        first.behavior_revision("direct").unwrap(),
        changed_unregistered.behavior_revision("direct").unwrap()
    );
}

#[test]
fn method_dependencies_use_resolved_receiver_identity() {
    let mut source = Source::parse_compact(
        "v1
         (rec Left value:i64)
         (rec Right value:i64)
         (fn Left.get (self:Left) i64 () (ret 1))
         (fn Right.get (self:Right) i64 () (ret 2))
         (pub choose () i64 () (ret (m (new Left 0) get)))",
    )
    .unwrap();
    let make = |source| Snapshot::new(source, vec![target("choose")]).unwrap();
    let base = make(source.clone());
    source
        .functions
        .iter_mut()
        .find(|function| function.receiver.as_deref() == Some("Right"))
        .unwrap()
        .body = vec![Stmt::Return {
        value: Expr::I64 { value: 8 },
    }];
    let unrelated = make(source.clone());
    assert_eq!(
        base.behavior_revision("choose").unwrap(),
        unrelated.behavior_revision("choose").unwrap()
    );
    source
        .functions
        .iter_mut()
        .find(|function| function.receiver.as_deref() == Some("Left"))
        .unwrap()
        .body = vec![Stmt::Return {
        value: Expr::I64 { value: 9 },
    }];
    let changed = make(source);
    assert_ne!(
        base.behavior_revision("choose").unwrap(),
        changed.behavior_revision("choose").unwrap()
    );
}

#[test]
fn explicit_assumptions_are_not_overwritten_and_unknown_roots_fail_closed() {
    let base = graph();
    let mut reader = checked(&base, "output", 1, &["caller"]);
    reader
        .change
        .requires
        .insert("leaf".into(), base.revision("aux").unwrap());
    let result = base.prepare_checked(&[reader]).unwrap();
    assert!(result.accepted.is_empty());
    assert!(
        matches!(result.rejected.values().next(), Some(Rejection::Stale(name)) if name == "leaf")
    );
    assert!(base.behavior_read_set(["missing"]).is_err());
    assert_eq!(base.behavior_revision("missing").unwrap(), None);
    let mut unknown = checked(&base, "output", 1, &[]);
    unknown.behavior_requires.insert(
        "missing".into(),
        base.behavior_revision("caller").unwrap().unwrap(),
    );
    let result = base.prepare_checked(&[unknown]).unwrap();
    assert!(result.accepted.is_empty());
    assert_eq!(result.snapshot().id(), base.id());
}

#[test]
fn profile_inputs_and_nominal_schema_changes_invalidate_behavioral_context() {
    let source = Source::parse_compact("v1 (rec Point x:i64) (pub f () i64 () (ret 1))").unwrap();
    let make = |source| Snapshot::new(source, vec![target("f")]).unwrap();
    let base = make(source.clone());
    let mut schema = source.clone();
    schema.records[0].fields[0].name = "y".into();
    let changed = make(schema);
    assert_eq!(base.revision("f"), changed.revision("f"));
    assert_ne!(
        base.behavior_revision("f").unwrap(),
        changed.behavior_revision("f").unwrap()
    );
    let mut input = source.clone();
    input.inputs.push(fluxr::structured::Binding {
        name: "clock".into(),
        ty: fluxr::structured::Type::I64,
    });
    assert_ne!(
        base.behavior_revision("f").unwrap(),
        make(input).behavior_revision("f").unwrap()
    );
    let mut profile = source;
    profile.version = 2;
    assert_ne!(
        base.behavior_revision("f").unwrap(),
        make(profile).behavior_revision("f").unwrap()
    );
}

#[test]
fn one_atomic_change_can_update_its_own_behavioral_dependencies() {
    let base = graph();
    let mut atomic = checked(&base, "output", 1, &["caller"]);
    atomic.change.edits.push(edit(&base, "leaf", 2));
    let result = base.prepare_checked(&[atomic]).unwrap();
    assert!(result.rejected.is_empty());
    assert_eq!(result.accepted.len(), 1);
    assert_ne!(result.snapshot().revision("leaf"), base.revision("leaf"));
    assert_ne!(
        result.snapshot().revision("output"),
        base.revision("output")
    );
}

#[test]
fn oversized_checked_envelopes_cannot_block_independent_work() {
    let base = graph();
    let mut huge = checked(&base, "output", 1, &[]);
    huge.change.author = "x".repeat(256 * 1024);
    let valid = checked(&base, "aux", 5, &[]);
    let result = base.prepare_checked(&[huge, valid]).unwrap();
    assert_eq!(result.accepted.len(), 1);
    assert_eq!(result.rejected.len(), 1);
    assert!(matches!(
        result.rejected.values().next(),
        Some(Rejection::Invalid(_))
    ));
}
