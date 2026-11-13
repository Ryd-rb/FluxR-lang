use fluxr::structured::authoring::{
    AuthoringDraft, BodyTarget, Cancellation, ErrorKind, QueryLimits,
};
use fluxr::structured::{Invocation, Limits, Module, Type, Value};
use std::sync::Arc;

const BASE: &str = "fa1 v2\npub calc 2 x int y int int ret div x y\nfn helper 1 x int int ret x";

fn draft(text: &str, holes: &[BodyTarget]) -> AuthoringDraft {
    AuthoringDraft::open_fa1(text, 7, [19; 32], holes, &Cancellation::default()).unwrap()
}

#[test]
fn typed_holes_never_admit_and_same_body_restores_exact_fsm() {
    let target = BodyTarget::function("calc");
    let draft = draft(BASE, std::slice::from_ref(&target));
    assert_eq!(draft.unresolved_count(), 1);
    assert_eq!(draft.original_source(), BASE);
    assert_eq!(
        draft
            .admit(draft.binding(), &Cancellation::default())
            .unwrap_err()
            .kind,
        ErrorKind::Unresolved
    );
    let context = draft
        .query(
            draft.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(context.expected_type(), &Type::I64);
    assert!(context.analysis_only());
    assert!(
        context
            .packet()
            .contains("effects unproven\nresources unproven")
    );
    assert!(context.packet().contains("capabilities unproven"));
    assert!(context.packet().contains("param x int"));
    assert!(!context.packet().contains("ret div"));
    let filled = draft
        .fill_body(&context, "fa1 body 1 ret div x y", &Cancellation::default())
        .unwrap();
    assert_eq!(filled.unresolved_count(), 0);
    assert_eq!(
        filled
            .admit(filled.binding(), &Cancellation::default())
            .unwrap()
            .encode(),
        Module::compile_agent(BASE).unwrap().encode()
    );
    assert_eq!(draft.unresolved_count(), 1);
}

#[test]
fn raw_revision_profile_compiler_semantic_and_sketch_bindings_are_separate() {
    let target = BodyTarget::function("calc");
    let original = draft(BASE, std::slice::from_ref(&target));
    let spaced = draft(&BASE.replace('\n', "\r\n  "), std::slice::from_ref(&target));
    assert_ne!(
        original.binding().source_sha256,
        spaced.binding().source_sha256
    );
    assert_ne!(
        original.binding().node_index_sha256,
        spaced.binding().node_index_sha256
    );
    assert_eq!(
        original.binding().base_semantic_sha256,
        spaced.binding().base_semantic_sha256
    );
    assert_eq!(
        original.binding().sketch_sha256,
        spaced.binding().sketch_sha256
    );
    assert_eq!(
        spaced
            .query(
                original.binding(),
                &target,
                QueryLimits::default(),
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::Stale
    );
    for binding in [
        {
            let mut b = original.binding().clone();
            b.compiler_sha256[0] ^= 1;
            b
        },
        {
            let mut b = original.binding().clone();
            b.buffer_version += 1;
            b
        },
        {
            let mut b = original.binding().clone();
            b.generation += 1;
            b
        },
        {
            let mut b = original.binding().clone();
            b.profile = "invented";
            b
        },
        {
            let mut b = original.binding().clone();
            b.compiler_contract = "invented";
            b
        },
        {
            let mut b = original.binding().clone();
            b.sketch_sha256[1] ^= 1;
            b
        },
    ] {
        assert_eq!(
            original
                .query(
                    &binding,
                    &target,
                    QueryLimits::default(),
                    &Cancellation::default()
                )
                .unwrap_err()
                .kind,
            ErrorKind::Stale
        );
    }
}

#[test]
fn every_fill_invalidates_earlier_contexts_without_mutating_other_forks() {
    let calc = BodyTarget::function("calc");
    let helper = BodyTarget::function("helper");
    let original = draft(BASE, &[calc.clone(), helper.clone()]);
    let c = original
        .query(
            original.binding(),
            &calc,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let h = original
        .query(
            original.binding(),
            &helper,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let first = original
        .fill_body(&c, "fa1 body 1 ret add x y", &Cancellation::default())
        .unwrap();
    assert_eq!(
        first
            .fill_body(&h, "fa1 body 1 ret x", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Stale
    );
    assert_eq!(
        first
            .admit(first.binding(), &Cancellation::default())
            .unwrap_err()
            .kind,
        ErrorKind::Unresolved
    );
    let h2 = first
        .query(
            first.binding(),
            &helper,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let ready = first
        .fill_body(&h2, "fa1 body 1 ret mul x 2", &Cancellation::default())
        .unwrap();
    assert!(
        ready
            .admit(ready.binding(), &Cancellation::default())
            .is_ok()
    );
    assert_eq!(original.unresolved_count(), 2);
    let other = original
        .fill_body(&c, "fa1 body 1 ret sub x y", &Cancellation::default())
        .unwrap();
    assert_ne!(first.binding().draft_sha256, other.binding().draft_sha256);
    assert_eq!(
        other
            .query(
                first.binding(),
                &helper,
                QueryLimits::default(),
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::Stale
    );
}

#[test]
fn ambiguous_names_require_receiver_and_none_is_not_an_absent_receiver() {
    let text = "fa1 v2 rec none 1 n int method none calc 1 self record none int ret field self n pub calc 0 int ret 5";
    let free = BodyTarget::function("calc");
    let method = BodyTarget::method("none", "calc");
    let d = draft(text, &[free.clone(), method.clone()]);
    assert_eq!(
        d.resolve_name("calc").unwrap_err().kind,
        ErrorKind::AmbiguousTarget
    );
    assert_eq!(
        d.resolve_name("missing").unwrap_err().kind,
        ErrorKind::UnknownTarget
    );
    let a = d
        .query(
            d.binding(),
            &free,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let b = d
        .query(
            d.binding(),
            &method,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(a.packet().contains("target function calc"));
    assert!(b.packet().contains("target method none calc"));
    assert_ne!(a.packet(), b.packet());
}

#[test]
fn malformed_unknown_and_duplicate_targets_fail_without_admission() {
    let cancel = Cancellation::default();
    let target = BodyTarget::function("calc");
    for (targets, kind) in [
        (vec![], ErrorKind::InvalidRequest),
        (
            vec![target.clone(), target.clone()],
            ErrorKind::InvalidRequest,
        ),
        (
            vec![BodyTarget::function("missing")],
            ErrorKind::UnknownTarget,
        ),
        (
            vec![BodyTarget::function("bad-name")],
            ErrorKind::InvalidRequest,
        ),
    ] {
        assert_eq!(
            AuthoringDraft::open_fa1(BASE, 0, [1; 32], &targets, &cancel)
                .err()
                .unwrap()
                .kind,
            kind
        );
    }
    assert_eq!(
        AuthoringDraft::open_fa1(BASE, 0, [0; 32], &[target], &cancel)
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        AuthoringDraft::open_fa1(
            "fa1 v2 pub broken",
            0,
            [1; 32],
            &[BodyTarget::function("broken")],
            &cancel
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::Parse
    );
    assert_eq!(
        AuthoringDraft::open_fa1(
            "fa1 v2 pub bad 0 int ret true",
            0,
            [1; 32],
            &[BodyTarget::function("bad")],
            &cancel
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::Type
    );
}

#[test]
fn syntax_and_type_failures_are_bounded_and_preserve_the_old_sketch() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let c = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    for (body, kind) in [
        ("fa1 body 1 ret true", ErrorKind::Type),
        ("fa1 body 1 ret hole", ErrorKind::Type),
        ("fa1 body 0", ErrorKind::Type),
        ("fa1 body 1 ret add x", ErrorKind::Parse),
        ("fa1 body 1 ret x extra", ErrorKind::Parse),
        ("fa1 body 1 ret tof x", ErrorKind::Type),
    ] {
        let e = d
            .fill_body(&c, body, &Cancellation::default())
            .err()
            .unwrap();
        assert_eq!(e.kind, kind, "{body}");
        assert!(e.detail().len() <= 240);
        assert_eq!(d.unresolved_count(), 1);
        assert_eq!(d.binding(), c.binding());
    }
    let oversized = "x".repeat(256 * 1024 + 1);
    assert_eq!(
        d.fill_body(&c, &oversized, &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        AuthoringDraft::open_fa1(&oversized, 0, [1; 32], &[target], &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn bounded_context_reports_omission_and_is_deterministic() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let full = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(!full.truncated());
    assert_eq!(full.visible_items(), full.total_items());
    assert_eq!(
        full.packet(),
        d.query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default()
        )
        .unwrap()
        .packet()
    );
    let zero = d
        .query(
            d.binding(),
            &target,
            QueryLimits {
                max_bytes: 2048,
                max_items: 0,
            },
            &Cancellation::default(),
        )
        .unwrap();
    assert!(zero.truncated());
    assert_eq!(zero.visible_items(), 0);
    assert_eq!(zero.total_items(), full.total_items());
    assert!(zero.packet().contains("context truncated"));
    for bytes in [1, 100, 500] {
        assert_eq!(
            d.query(
                d.binding(),
                &target,
                QueryLimits {
                    max_bytes: bytes,
                    max_items: 1
                },
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::OutputLimit
        );
    }
    for limits in [
        QueryLimits {
            max_bytes: 0,
            max_items: 1,
        },
        QueryLimits {
            max_bytes: 65537,
            max_items: 1,
        },
        QueryLimits {
            max_bytes: 1024,
            max_items: 4097,
        },
    ] {
        assert_eq!(
            d.query(d.binding(), &target, limits, &Cancellation::default())
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
    }
}

#[test]
fn cancellation_is_terminal_and_request_local() {
    let target = BodyTarget::function("calc");
    let d = Arc::new(draft(BASE, std::slice::from_ref(&target)));
    let c = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let token = Cancellation::default();
    let clone = token.clone();
    token.cancel();
    assert!(clone.is_cancelled());
    assert_eq!(
        AuthoringDraft::open_fa1(BASE, 0, [1; 32], std::slice::from_ref(&target), &token)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        d.query(d.binding(), &target, QueryLimits::default(), &token)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        d.fill_body(&c, "fa1 body 1 ret x", &token)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        d.admit(d.binding(), &token).unwrap_err().kind,
        ErrorKind::Cancelled
    );
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let d = Arc::clone(&d);
            let target = target.clone();
            std::thread::spawn(move || {
                d.query(
                    d.binding(),
                    &target,
                    QueryLimits::default(),
                    &Cancellation::default(),
                )
                .unwrap()
                .packet()
                .to_owned()
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), c.packet());
    }
}

#[test]
fn filled_program_preserves_direct_compiler_first_traps_and_every_budget_outcome() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let c = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let d = d
        .fill_body(&c, "fa1 body 1 ret div add x 1 y", &Cancellation::default())
        .unwrap();
    let module = d.admit(d.binding(), &Cancellation::default()).unwrap();
    let direct = Module::compile_agent(&BASE.replace("div x y", "div add x 1 y")).unwrap();
    assert_eq!(module.encode(), direct.encode());
    for (x, y) in [(3, 2), (i64::MAX, 0), (1, 0), (i64::MIN, -1)] {
        for instructions in 0..16 {
            for cells in 0..8 {
                for frames in 0..3 {
                    let run = |m: &Module| {
                        format!(
                            "{:?}",
                            m.run(Invocation {
                                export: "calc",
                                inputs: &[],
                                arguments: &[Value::I64(x), Value::I64(y)],
                                limits: Limits {
                                    instructions,
                                    cells,
                                    frames
                                },
                            })
                        )
                    };
                    assert_eq!(run(&module), run(&direct));
                }
            }
        }
    }
}

#[test]
fn completed_list_body_preserves_shared_input_copy_on_write() {
    let target = BodyTarget::function("trim");
    let d = draft(
        "fa1 v2 pub trim 1 xs list int list int ret xs",
        std::slice::from_ref(&target),
    );
    let c = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let d = d
        .fill_body(&c, "fa1 body 2 pop xs ret xs", &Cancellation::default())
        .unwrap();
    let module = d.admit(d.binding(), &Cancellation::default()).unwrap();
    let original = Arc::new(vec![Value::I64(7), Value::I64(8)]);
    let arguments = [Value::List(Arc::clone(&original))];
    assert_eq!(
        module
            .run(Invocation {
                export: "trim",
                inputs: &[],
                arguments: &arguments,
                limits: Limits::default()
            })
            .unwrap(),
        Value::List(Arc::new(vec![Value::I64(7)]))
    );
    assert_eq!(*original, vec![Value::I64(7), Value::I64(8)]);
}

#[test]
fn final_whole_module_budget_cannot_be_bypassed_by_locally_valid_hole_fills() {
    let mut base = String::from("fa1 v2\n");
    let mut targets = Vec::new();
    for index in 0..128 {
        base.push_str(&format!("pub f{index} 0 list int ret list int 0\n"));
        targets.push(BodyTarget::function(format!("f{index}")));
    }
    let mut d = draft(&base, &targets);
    let body = format!("fa1 body 1 ret list int 256 {}", vec!["1"; 256].join(" "));
    for target in &targets {
        let c = d
            .query(
                d.binding(),
                target,
                QueryLimits {
                    max_bytes: 2048,
                    max_items: 0,
                },
                &Cancellation::default(),
            )
            .unwrap();
        d = d.fill_body(&c, &body, &Cancellation::default()).unwrap();
    }
    assert_eq!(d.unresolved_count(), 0);
    assert_eq!(
        d.admit(d.binding(), &Cancellation::default())
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn oversized_signature_context_keeps_exact_counts_and_never_returns_a_partial_item() {
    let fields = (0..256)
        .map(|i| format!("f{i:03}{} int", "x".repeat(60)))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "fa1 v2 rec Huge 256 {fields}\npub calc 0 int ret 0\nfn large 256 {fields} int ret 0"
    );
    let target = BodyTarget::function("calc");
    let d = draft(&source, std::slice::from_ref(&target));
    for max_items in [0, 100] {
        let c = d
            .query(
                d.binding(),
                &target,
                QueryLimits {
                    max_bytes: 1024,
                    max_items,
                },
                &Cancellation::default(),
            )
            .unwrap();
        assert!(c.truncated());
        assert_eq!(c.total_items(), 3);
        assert_eq!(c.visible_items(), 0);
        assert!(c.packet().len() <= 1024);
        assert!(!c.packet().contains("rec Huge"));
        assert!(!c.packet().contains("fn large"));
    }
    let full = d
        .query(
            d.binding(),
            &target,
            QueryLimits {
                max_bytes: 65536,
                max_items: 4096,
            },
            &Cancellation::default(),
        )
        .unwrap();
    assert!(!full.truncated());
    assert_eq!(full.visible_items(), 3);
    assert_eq!(full.total_items(), 3);
    assert!(full.packet().contains("rec Huge 256"));
    assert!(full.packet().contains("fn large 256"));
}

#[test]
fn equivalent_structural_fills_share_a_draft_identity_without_claiming_raw_edit_identity() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let c = d
        .query(
            d.binding(),
            &target,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let first = d
        .fill_body(&c, "fa1 body 1 ret add x y", &Cancellation::default())
        .unwrap();
    let spaced = d
        .fill_body(
            &c,
            "fa1\r\nbody  1\tret add  x  y\n",
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(first.binding(), spaced.binding());
    assert_eq!(first.original_source(), BASE);
    assert_eq!(
        first
            .admit(first.binding(), &Cancellation::default())
            .unwrap()
            .encode(),
        spaced
            .admit(spaced.binding(), &Cancellation::default())
            .unwrap()
            .encode()
    );
}
