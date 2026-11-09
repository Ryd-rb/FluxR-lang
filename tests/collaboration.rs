use fluxr::collaboration::{
    Change, Edit, LiveProgram, Mode, Rejection, Snapshot, Target, TestRun, UnitCase,
};
use fluxr::structured::{Expr, Limits, Source, Stmt, Value};

fn snapshot() -> Snapshot {
    let source = Source::parse_compact(
        "v1
        (pub choice () u64 () (ret 0u))
        (pub aux () u64 () (ret 0u))
        (pub slope () f32 () (ret 1f))",
    )
    .unwrap();
    let target = |id: &str, function: &str, mode| Target {
        id: id.into(),
        function: function.into(),
        receiver: None,
        mode,
    };
    Snapshot::new(
        source,
        vec![
            target("id-choice", "choice", Mode::Exclusive),
            target("id-aux", "aux", Mode::Exclusive),
            target("id-slope", "slope", Mode::MinF32),
        ],
    )
    .unwrap()
}

fn edit(snapshot: &Snapshot, target: &str, value: Expr) -> Edit {
    Edit::ReplaceBody {
        target: target.into(),
        expected: snapshot.edit_revision(target).unwrap(),
        body: vec![Stmt::Return { value }],
    }
}

fn change(author: &str, edits: Vec<Edit>) -> Change {
    Change {
        author: author.into(),
        requires: Default::default(),
        edits,
    }
}

fn limit(snapshot: &Snapshot, author: &str, value: f32) -> Change {
    change(
        author,
        vec![Edit::MinF32 {
            target: "id-slope".into(),
            expected: snapshot.edit_revision("id-slope").unwrap(),
            bits: value.to_bits(),
        }],
    )
}

fn assertion(snapshot: &Snapshot, export: &str, expected: Value) -> UnitCase {
    UnitCase {
        id: export.into(),
        snapshot: snapshot.id(),
        export: export.into(),
        inputs: vec![],
        arguments: vec![],
        limits: Limits::default(),
        expected,
    }
}

#[test]
fn identical_retries_and_late_reducer_contributions_converge() {
    let base = snapshot();
    let left = limit(&base, "left", 0.8);
    let right = limit(&base, "right", 0.5);
    let batch = base
        .prepare(&[right.clone(), left.clone(), right.clone()])
        .unwrap();
    assert_eq!(batch.accepted.len(), 2);
    let mut live = LiveProgram::new(base.clone());
    live.activate(base.prepare(&[left]).unwrap(), 5).unwrap();
    let next = live.snapshot().prepare(&[right]).unwrap();
    let receipt = live.activate(next, 6).unwrap();
    assert_eq!(receipt.after, batch.snapshot().id());
    assertion(live.snapshot(), "slope", Value::F32(0.5_f32.to_bits()))
        .run(live.snapshot(), Limits::default())
        .unwrap();
}

#[test]
fn incompatible_bodies_hold_the_whole_batch_and_invalid_edits_cannot_block_valid_edits() {
    let base = snapshot();
    let a = change(
        "a",
        vec![
            edit(&base, "id-choice", Expr::U64 { value: 1 }),
            edit(&base, "id-aux", Expr::U64 { value: 1 }),
        ],
    );
    let b = change("b", vec![edit(&base, "id-choice", Expr::U64 { value: 2 })]);
    let c = change(
        "c",
        vec![edit(&base, "id-choice", Expr::Bool { value: false })],
    );
    let d = limit(&base, "d", 0.5);
    let mixed = base.prepare(&[a.clone(), b, c.clone(), d]).unwrap();
    assert_eq!(mixed.accepted.len(), 1);
    assert_eq!(
        mixed
            .rejected
            .values()
            .filter(|e| matches!(e, Rejection::Conflict(_)))
            .count(),
        2
    );
    assert_eq!(
        mixed
            .rejected
            .values()
            .filter(|e| matches!(e, Rejection::Invalid(_)))
            .count(),
        1
    );
    assertion(mixed.snapshot(), "aux", Value::U64(0))
        .run(mixed.snapshot(), Limits::default())
        .unwrap();
    let unblocked = base.prepare(&[a, c]).unwrap();
    assert_eq!(unblocked.accepted.len(), 1);
    assertion(unblocked.snapshot(), "choice", Value::U64(1))
        .run(unblocked.snapshot(), Limits::default())
        .unwrap();
}

#[test]
fn stale_assumptions_and_activation_preserve_the_admitted_program() {
    let base = snapshot();
    let a = change("a", vec![edit(&base, "id-choice", Expr::U64 { value: 1 })]);
    let mut b = change("b", vec![edit(&base, "id-aux", Expr::U64 { value: 2 })]);
    b.requires
        .insert("id-choice".into(), base.revision("id-choice").unwrap());
    let mixed = base.prepare(&[a.clone(), b.clone()]).unwrap();
    assert!(matches!(
        mixed.rejected.values().next(),
        Some(Rejection::DependencyChanged(_))
    ));
    let later = base.prepare(&[b.clone()]).unwrap();
    let mut live = LiveProgram::new(base);
    live.activate(mixed, 1).unwrap();
    let id = live.snapshot().id();
    assert!(live.activate(later, 2).is_err());
    assert_eq!(live.snapshot().id(), id);
    assert!(matches!(
        live.snapshot()
            .prepare(&[a, b])
            .unwrap()
            .rejected
            .values()
            .next(),
        Some(Rejection::Stale(_))
    ));
}

#[test]
fn unit_and_stateful_test_forks_do_not_share_their_active_program_or_mutable_state() {
    let base = snapshot();
    let fixture = vec![100];
    std::thread::scope(|scope| {
        let runs: Vec<_> = (0..10)
            .map(|id| {
                let fixture = &fixture;
                let base = &base;
                scope.spawn(move || {
                    let candidate = base
                        .prepare(&[change(
                            "agent",
                            vec![edit(base, "id-choice", Expr::U64 { value: id })],
                        )])
                        .unwrap();
                    let mut run = TestRun::fork(fixture, base);
                    let receipt = run.activate(candidate, 1).unwrap();
                    let unit = assertion(run.snapshot(), "choice", Value::U64(id));
                    let digest = unit.run(run.snapshot(), Limits::default()).unwrap();
                    assert_eq!(digest, unit.run(run.snapshot(), Limits::default()).unwrap());
                    assert!(
                        unit.run(
                            run.snapshot(),
                            Limits {
                                instructions: 0,
                                ..Limits::default()
                            }
                        )
                        .is_err()
                    );
                    if id != 0 {
                        assert!(unit.run(base, Limits::default()).is_err());
                    }
                    run.step(|state, snapshot| {
                        assert_eq!(snapshot.id(), receipt.after);
                        state.push(id);
                    });
                    assert_eq!(run.state(), &[100, id]);
                })
            })
            .collect();
        for run in runs {
            run.join().unwrap();
        }
    });
    assert_eq!(fixture, [100]);
}

#[test]
fn relinked_caller_dispatches_to_the_new_body_and_matches_full_decode() {
    let mut source = Source::parse_compact(
        "v1
        (pub choice () u64 () (ret 0u))
        (pub caller () u64 () (ret (call choice)))",
    )
    .unwrap();
    let target = Target {
        id: "choice".into(),
        function: "choice".into(),
        receiver: None,
        mode: Mode::Exclusive,
    };
    let base = Snapshot::new(source.clone(), vec![target]).unwrap();
    let body = vec![Stmt::Return {
        value: Expr::U64 { value: 42 },
    }];
    let candidate = base
        .prepare(&[change(
            "edit",
            vec![Edit::ReplaceBody {
                target: "choice".into(),
                expected: base.edit_revision("choice").unwrap(),
                body: body.clone(),
            }],
        )])
        .unwrap();
    source.functions[0].body = body;
    let full = fluxr::structured::Module::compile(source).unwrap();
    assert_eq!(candidate.snapshot().module().encode(), full.encode());
    assertion(candidate.snapshot(), "caller", Value::U64(42))
        .run(candidate.snapshot(), Limits::default())
        .unwrap();
    assertion(&base, "caller", Value::U64(0))
        .run(&base, Limits::default())
        .unwrap();
}

#[test]
fn unchanged_body_does_not_compete_with_an_edit() {
    let base = snapshot();
    let review = change(
        "review",
        vec![edit(&base, "id-choice", Expr::U64 { value: 0 })],
    );
    let update = change(
        "update",
        vec![edit(&base, "id-choice", Expr::U64 { value: 2 })],
    );
    let candidate = base.prepare(&[review, update]).unwrap();
    assert_eq!(candidate.accepted.len(), 2);
    assert!(candidate.rejected.is_empty());
    assertion(candidate.snapshot(), "choice", Value::U64(2))
        .run(candidate.snapshot(), Limits::default())
        .unwrap();
}

#[test]
fn oversized_proposal_does_not_hold_an_independent_change() {
    let base = snapshot();
    let large = change(
        &"x".repeat(260 * 1024),
        vec![edit(&base, "id-choice", Expr::U64 { value: 1 })],
    );
    let candidate = base.prepare(&[large, limit(&base, "valid", 0.5)]).unwrap();
    assert_eq!(candidate.accepted.len(), 1);
    assert!(matches!(
        candidate.rejected.values().next(),
        Some(Rejection::Invalid(_))
    ));
}
