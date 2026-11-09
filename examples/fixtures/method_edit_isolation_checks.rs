//! Verify collaboration edits and slot publication with the ordered-call ledger.
use super::{
    CELLS, Case, GOOD, INDEXED, MATH, POST_WRITE, PROJECTED, SOURCE, check_aliases, check_result,
    limits, point,
};
use fluxr::collaboration::{Change, Edit, LiveProgram, Mode, Rejection, Snapshot, Target};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Source, Stmt, Value, Workspace};

const POINT: &str = "Point.update";
const TWIN: &str = "Twin.update";
const EXPORTS: [(&str, bool, Limits); 2] =
    [("indexed", false, INDEXED), ("projected", true, PROJECTED)];

fn target(id: &str, receiver: Option<&str>, function: &str) -> Target {
    Target {
        id: id.into(),
        receiver: receiver.map(str::to_owned),
        function: function.into(),
        mode: Mode::Exclusive,
    }
}

fn body(source: &Source, receiver: &str) -> Vec<Stmt> {
    source
        .functions
        .iter()
        .find(|f| f.receiver.as_deref() == Some(receiver) && f.name == "update")
        .unwrap()
        .body
        .clone()
}

fn edit(base: &Snapshot, target: &str, body: Vec<Stmt>) -> Edit {
    Edit::ReplaceBody {
        target: target.into(),
        expected: base.edit_revision(target).unwrap(),
        body,
    }
}

fn change(edits: Vec<Edit>) -> Change {
    Change {
        author: "fixture".into(),
        requires: Default::default(),
        edits,
    }
}

fn reject(base: &Snapshot, change: Change, reason: Rejection) {
    let candidate = base.prepare(&[change]).unwrap();
    assert!(candidate.accepted.is_empty());
    // The rejected proposal must reuse the retained snapshot.
    assert_eq!(candidate.snapshot().id(), base.id());
    assert_eq!(
        candidate.snapshot().module().encode(),
        base.module().encode()
    );
    assert_eq!(
        candidate.rejected.into_values().collect::<Vec<_>>(),
        [reason]
    );
}

fn call<'a>(
    export: &'a str,
    inputs: &'a [Value],
    args: &'a [Value],
    cap: Limits,
) -> Invocation<'a> {
    Invocation {
        export,
        inputs,
        arguments: args,
        limits: cap,
    }
}

fn check_output(
    result: Result<Value, fluxr::Error>,
    expected: &Result<Value, &str>,
    snapshot: &Value,
) {
    let result = result.map_err(|error| error.to_string());
    assert_eq!(
        result.as_ref().map_err(String::as_str),
        expected.as_ref().map_err(|&trap| trap)
    );
    if let Ok(value) = &result {
        check_result(value, snapshot);
    }
}

fn publish(slot: &mut ModuleSlot, next: Module, incompatible: &Module, old: &[u8]) {
    let inputs = GOOD.inputs();
    let good = GOOD.args(false, &inputs[1]);
    let failing = POST_WRITE.args(false, &inputs[1]);
    let exact = || call("indexed", &inputs, &good, INDEXED);
    let post_write = || call("indexed", &inputs, &failing, limits(18, 21, 2));
    let new = next.encode();
    for (replacement, probe, trap) in [
        (
            incompatible.clone(),
            post_write(),
            "replacement module has a different host interface",
        ),
        (next.clone(), post_write(), MATH),
        (
            next.clone(),
            call("indexed", &inputs, &good, limits(24, 30, 2)),
            CELLS,
        ),
    ] {
        assert_eq!(
            slot.inject(0, replacement, Some(probe))
                .unwrap_err()
                .to_string(),
            trap
        );
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), old);
        check_output(
            slot.run(exact()),
            &GOOD.expected(false, INDEXED, false),
            &inputs[1],
        );
        check_aliases(GOOD, false, &inputs, &good);
    }
    assert_eq!(slot.inject(0, next, Some(exact())).unwrap(), 1);
    assert_eq!(
        slot.inject(0, Module::decode(old).unwrap(), Some(post_write()))
            .unwrap_err()
            .to_string(),
        "module revision changed"
    );
    assert_eq!(slot.revision(), 1);
    assert_eq!(slot.module().encode(), new);
    check_output(
        slot.run(exact()),
        &GOOD.expected(false, INDEXED, true),
        &inputs[1],
    );
    check_aliases(GOOD, false, &inputs, &good);
}

fn ledger(snapshots: [&Snapshot; 2], slots: [&[ModuleSlot; 2]; 2]) -> usize {
    let mut workspace = Workspace::default();
    let mut checked = 0;
    for (export, projected, exact) in EXPORTS {
        let shift = u64::from(projected);
        let admission = 3 * usize::from(projected);
        let cases = [
            (GOOD, exact),
            (GOOD, limits(exact.instructions - 1, exact.cells, 2)),
            (GOOD, limits(exact.instructions, exact.cells - 1, 2)),
            (GOOD, limits(8 - shift, 15 + admission, 1)),
            (GOOD, limits(9 - shift, 15 + admission, 1)),
            (GOOD, limits(9 - shift, 18 + admission, 2)),
            (GOOD, limits(14 - shift, 20 + admission, 2)),
            (
                Case {
                    divisor: 0,
                    scale: None,
                    ..GOOD
                },
                limits(6 - shift, exact.cells, 2),
            ),
            (
                Case {
                    scale: None,
                    ..GOOD
                },
                limits(8 - shift, exact.cells, 2),
            ),
            (POST_WRITE, limits(17 - shift, 21 + admission, 2)),
            (POST_WRITE, limits(18 - shift, 21 + admission, 2)),
            (
                Case {
                    initial: i64::MAX,
                    ..GOOD
                },
                exact,
            ),
            (
                Case {
                    initial: i64::MIN,
                    ..GOOD
                },
                exact,
            ),
        ];
        let bounds = (
            Case {
                index: 1,
                divisor: 0,
                scale: None,
                ..GOOD
            },
            limits(3, 14, 1),
        );
        for (case, cap) in cases.into_iter().chain((!projected).then_some(bounds)) {
            let inputs = case.inputs();
            let args = case.args(projected, &inputs[1]);
            let invocation = || call(export, &inputs, &args, cap);
            for (index, (snapshot, routes)) in snapshots.iter().zip(slots).enumerate() {
                let module = snapshot.module();
                let expected = case.expected(projected, cap, index == 1);
                for result in [
                    module.run_with(invocation(), &mut workspace),
                    module.run_portable_with(invocation(), &mut workspace),
                    routes[1]
                        .module()
                        .run_portable_with(invocation(), &mut workspace),
                    routes[0].run_with(invocation(), &mut workspace),
                    routes[1].run_with(invocation(), &mut workspace),
                ] {
                    check_output(result, &expected, &inputs[1]);
                    check_aliases(case, projected, &inputs, &args);
                }
                checked += 1;
            }
        }
    }
    checked
}

#[cfg(not(target_arch = "wasm32"))]
fn overlap(old: &Snapshot, active: &Snapshot) {
    use std::sync::Barrier;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    let inputs = GOOD.inputs();
    let args = [GOOD.args(false, &inputs[1]), GOOD.args(true, &inputs[1])];
    let start = Barrier::new(2);
    let (old_done, old_round) = sync_channel(1);
    let (new_done, new_round) = sync_channel(1);
    let replay = |snapshot: &Snapshot, subtract, done: SyncSender<()>, peer: Receiver<()>| {
        let mut workspace = Workspace::default();
        start.wait();
        for _ in 0..64 {
            for ((export, projected, cap), args) in EXPORTS.into_iter().zip(&args) {
                check_output(
                    snapshot
                        .module()
                        .run_with(call(export, &inputs, args, cap), &mut workspace),
                    &GOOD.expected(projected, cap, subtract),
                    &inputs[1],
                );
            }
            // Keep both revision runners in the same round. If either
            // assertion fails, dropping its channel ends the other runner.
            done.send(()).unwrap();
            peer.recv().unwrap();
        }
    };
    std::thread::scope(|scope| {
        let replay = &replay;
        let retained = scope.spawn(move || replay(old, false, old_done, new_round));
        replay(active, true, new_done, old_round);
        retained.join().unwrap();
    });
    for ((_, projected, _), args) in EXPORTS.into_iter().zip(&args) {
        check_aliases(GOOD, projected, &inputs, args);
    }
}

pub(crate) fn check_method_edit_isolation() -> usize {
    let text = format!("{SOURCE}\n{}", include_str!("method_edit_identity.fluxr"));
    let edited = text.replace("(+ self.value delta)", "(- self.value delta)");
    let original = Source::parse_compact(&text).unwrap();
    let next = Source::parse_compact(&edited).unwrap();
    let grants = &original.inputs;
    let twin_body = body(&original, "Twin");
    assert_eq!(
        Snapshot::new(
            original.clone(),
            vec![target(POINT, Some("Holder"), "update")]
        )
        .err()
        .unwrap()
        .to_string(),
        "unknown declaration update"
    );
    let base = Snapshot::new(
        original.clone(),
        vec![
            target(POINT, Some("Point"), "update"),
            target(TWIN, Some("Twin"), "update"),
            target("indexed", None, "indexed"),
            target("projected", None, "projected"),
        ],
    )
    .unwrap();
    let old = base.module().encode();
    let full = Module::compile(next.clone()).unwrap();
    let replacement = full.encode();
    assert_ne!(replacement, old);
    assert_ne!(base.revision(POINT), base.revision(TWIN));
    let update = edit(&base, POINT, body(&next, "Point"));
    let wrong_pin = Edit::ReplaceBody {
        target: POINT.into(),
        expected: base.edit_revision(TWIN).unwrap(),
        body: body(&next, "Point"),
    };
    reject(
        &base,
        change(vec![wrong_pin]),
        Rejection::Stale(POINT.into()),
    );
    let wrong_result =
        Source::parse_compact(&text.replace("(new Twin 13", "(new Point 13")).unwrap();
    reject(
        &base,
        change(vec![
            update.clone(),
            edit(&base, TWIN, body(&wrong_result, "Twin")),
        ]),
        Rejection::Invalid(r#"Twin.update: expected Record("Twin"), got Record("Point")"#.into()),
    );
    let update = change(vec![update]);
    let candidate = base.prepare(std::slice::from_ref(&update)).unwrap();
    let stale = base.prepare(std::slice::from_ref(&update)).unwrap();
    assert_eq!(candidate.accepted.len(), 1);
    assert!(candidate.rejected.is_empty());
    assert_eq!(candidate.snapshot().module().encode(), replacement);
    let mut live = LiveProgram::new(base.clone());
    let receipt = live.activate(candidate, 7).unwrap();
    let active = live.snapshot();
    assert_eq!(
        (receipt.tick, receipt.before, receipt.after),
        (7, base.id(), active.id())
    );
    assert_ne!(receipt.before, receipt.after);
    assert_ne!(base.revision(POINT), active.revision(POINT));
    for id in [TWIN, "indexed", "projected"] {
        assert_eq!(base.revision(id), active.revision(id));
    }
    reject(active, update, Rejection::Stale(POINT.into()));
    let mut dependent = change(vec![edit(active, TWIN, twin_body)]);
    dependent
        .requires
        .insert(POINT.into(), base.revision(POINT).unwrap());
    reject(active, dependent, Rejection::Stale(POINT.into()));
    assert_eq!(
        live.activate(stale, 8).unwrap_err().to_string(),
        "candidate was prepared against an older active snapshot"
    );
    let active = live.snapshot();
    assert_eq!(active.id(), receipt.after);
    assert_eq!(active.module().encode(), replacement);
    assert_eq!(base.module().encode(), old);

    let incompatible = Module::compile_compact(&edited.replace("Point", "OtherPoint")).unwrap();
    let old_source = Module::compile(original.clone()).unwrap();
    assert_eq!(old_source.encode(), old);
    let old_slots = [
        ModuleSlot::new(old_source.clone(), grants).unwrap(),
        ModuleSlot::new(Module::decode(&old).unwrap(), grants).unwrap(),
    ];
    let mut slots = [
        ModuleSlot::new(old_source, grants).unwrap(),
        ModuleSlot::new(Module::decode(&old).unwrap(), grants).unwrap(),
    ];
    publish(&mut slots[0], full, &incompatible, &old);
    publish(
        &mut slots[1],
        Module::decode(&replacement).unwrap(),
        &incompatible,
        &old,
    );

    for snapshot in [&base, active] {
        for (export, _, _) in EXPORTS {
            assert!(!snapshot.module().prepare_native(export));
            assert!(snapshot.module().emit_native_rust(export).is_err());
        }
        let inputs = GOOD.inputs();
        let twin = [point(7, 60)];
        assert_eq!(
            snapshot
                .module()
                .run(call("twin", &inputs, &twin, Limits::default()))
                .unwrap(),
            point(13, 60)
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    overlap(&base, active);
    ledger([&base, active], [&old_slots, &slots])
}
