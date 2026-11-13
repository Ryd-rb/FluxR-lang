use fluxr::structured::authoring::{
    AuthoringDraft, BodyTarget, COMPACT_CONTEXT_PROFILE, Cancellation, ErrorKind, PROFILE,
    QueryLimits, SignatureSelection,
};
use fluxr::structured::{Module, Type};

const BASE: &str = "fa1 v2
inputs 1 seed int
rec P 1 n int
rec Q 1 q bool
enum E 2 Empty none Some int
method P get 1 self record P int ret field self n
fn helper 1 z int int ret add z 3
pub calc 2 x int y int int do 1 tmp int 2 set tmp add x y ret tmp
fn later 0 int ret 17
";

fn draft(text: &str) -> AuthoringDraft {
    AuthoringDraft::open_fa1(
        text,
        7,
        [19; 32],
        &[BodyTarget::function("calc")],
        &Cancellation::default(),
    )
    .unwrap()
}

fn none() -> SignatureSelection<'static> {
    SignatureSelection::Explicit {
        records: &[],
        enums: &[],
        callables: &[],
    }
}

fn compact(
    draft: &AuthoringDraft,
    selection: SignatureSelection<'_>,
    limits: QueryLimits,
) -> fluxr::structured::authoring::BodyContext {
    draft
        .query_compact(
            draft.binding(),
            &BodyTarget::function("calc"),
            selection,
            limits,
            &Cancellation::default(),
        )
        .unwrap()
}

#[test]
fn all_signatures_preserve_full_ticket_and_exact_artifact_with_legacy_packet_unchanged() {
    let d = draft(BASE);
    let target = BodyTarget::function("calc");
    let cancel = Cancellation::default();
    let legacy = d
        .query(d.binding(), &target, QueryLimits::default(), &cancel)
        .unwrap();
    let c = compact(&d, SignatureSelection::All, QueryLimits::default());
    assert_eq!(c.binding(), legacy.binding());
    assert_eq!(c.target(), legacy.target());
    assert_eq!(c.expected_type(), &Type::I64);
    assert_eq!(c.packet_profile(), COMPACT_CONTEXT_PROFILE);
    assert_eq!(legacy.packet_profile(), PROFILE);
    assert_eq!(legacy.selected_items(), legacy.total_items());
    assert_eq!(legacy.unselected_items(), 0);
    assert_eq!(
        (c.visible_items(), c.selected_items(), c.total_items()),
        (11, 11, 11)
    );
    assert_eq!(c.unselected_items(), 0);
    assert!(!c.truncated());
    assert!(c.analysis_only());
    assert!(
        c.packet()
            .starts_with("ac1 analysis_only\nbinding host_ticket\ndraft ")
    );
    let hex: String = d
        .binding()
        .draft_sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(c.packet().contains(&format!("draft {hex}\n")));
    assert!(c.packet().contains("expected int\neffects unproven\nresources unproven\ncapabilities unproven\nrequires whole_module_check invocation_limits\n"));
    assert!(
        c.packet().ends_with(
            "items 11 11 11\nselection all\ncontext selected_complete\nadmission none\n"
        )
    );
    assert!(!c.packet().contains("\nsource "));
    assert!(c.packet().len() < legacy.packet().len());
    let signatures = |packet: &str| -> Vec<String> {
        packet
            .lines()
            .filter(|line| {
                [
                    "param ", "local ", "input ", "rec ", "enum ", "fn ", "method ",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            })
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(signatures(c.packet()), signatures(legacy.packet()));
    for ticket in [&legacy, &c] {
        let filled = d
            .fill_body(ticket, "fa1 body 2 set tmp add x y ret tmp", &cancel)
            .unwrap();
        assert_eq!(
            filled.admit(filled.binding(), &cancel).unwrap().encode(),
            Module::compile_agent(BASE).unwrap().encode()
        );
    }
    assert_eq!(
        legacy.packet(),
        d.query(d.binding(), &target, QueryLimits::default(), &cancel)
            .unwrap()
            .packet()
    );
    assert_eq!(d.unresolved_count(), 1);
}

#[test]
fn explicit_selection_keeps_current_scope_and_distinguishes_unselected_inventory() {
    let d = draft(BASE);
    let callables = [BodyTarget::function("helper")];
    let c = compact(
        &d,
        SignatureSelection::Explicit {
            records: &["P"],
            enums: &["E"],
            callables: &callables,
        },
        QueryLimits::default(),
    );
    assert_eq!(
        (
            c.visible_items(),
            c.selected_items(),
            c.total_items(),
            c.unselected_items()
        ),
        (7, 7, 11, 4)
    );
    assert!(!c.truncated());
    for line in [
        "param x int\n",
        "param y int\n",
        "local tmp int\n",
        "input seed int\n",
        "rec P 1 n int\n",
        "enum E 2 Empty none Some int\n",
        "fn helper 1 z int int\n",
    ] {
        assert!(c.packet().contains(line), "{line}");
    }
    for absent in ["rec Q ", "method P get", "fn calc ", "fn later ", "ret add"] {
        assert!(!c.packet().contains(absent), "{absent}");
    }
    assert!(
        c.packet()
            .contains("items 7 7 11\nselection explicit\ncontext selected_complete")
    );
}

#[test]
fn selector_order_does_not_change_deterministic_source_order() {
    let d = draft(BASE);
    let a = [
        BodyTarget::function("helper"),
        BodyTarget::function("later"),
    ];
    let b = [
        BodyTarget::function("later"),
        BodyTarget::function("helper"),
    ];
    let first = compact(
        &d,
        SignatureSelection::Explicit {
            records: &["Q", "P"],
            enums: &["E"],
            callables: &a,
        },
        QueryLimits::default(),
    );
    let second = compact(
        &d,
        SignatureSelection::Explicit {
            records: &["P", "Q"],
            enums: &["E"],
            callables: &b,
        },
        QueryLimits::default(),
    );
    assert_eq!(first.packet(), second.packet());
    assert!(first.packet().find("rec P ").unwrap() < first.packet().find("rec Q ").unwrap());
    assert!(first.packet().find("fn helper ").unwrap() < first.packet().find("fn later ").unwrap());
}

#[test]
fn mandatory_scope_is_complete_or_query_refuses_even_for_empty_selection() {
    let d = draft(BASE);
    for max_items in 0..4 {
        let error = d
            .query_compact(
                d.binding(),
                &BodyTarget::function("calc"),
                none(),
                QueryLimits {
                    max_bytes: 4096,
                    max_items,
                },
                &Cancellation::default(),
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::OutputLimit);
    }
    let c = compact(
        &d,
        none(),
        QueryLimits {
            max_bytes: 4096,
            max_items: 4,
        },
    );
    assert_eq!(
        (
            c.visible_items(),
            c.selected_items(),
            c.total_items(),
            c.unselected_items()
        ),
        (4, 4, 11, 7)
    );
    assert!(!c.truncated());
    assert_eq!(
        d.query_compact(
            d.binding(),
            &BodyTarget::function("calc"),
            none(),
            QueryLimits {
                max_bytes: 1,
                max_items: 4096
            },
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::OutputLimit
    );
}

#[test]
fn requested_truncation_is_distinct_from_deliberately_unselected_inventory() {
    let d = draft(BASE);
    let callables = [BodyTarget::function("helper")];
    let c = compact(
        &d,
        SignatureSelection::Explicit {
            records: &["P"],
            enums: &["E"],
            callables: &callables,
        },
        QueryLimits {
            max_bytes: 4096,
            max_items: 4,
        },
    );
    assert_eq!(
        (
            c.visible_items(),
            c.selected_items(),
            c.total_items(),
            c.unselected_items()
        ),
        (4, 7, 11, 4)
    );
    assert!(c.truncated());
    assert!(
        c.packet()
            .contains("items 4 7 11\nselection explicit\ncontext truncated")
    );
    assert!(!c.packet().contains("rec P "));
}

#[test]
fn all_selectors_are_validated_before_header_or_item_truncation() {
    fn test(d: &AuthoringDraft, selection: SignatureSelection<'_>, kind: ErrorKind) {
        let error = d
            .query_compact(
                d.binding(),
                &BodyTarget::function("calc"),
                selection,
                QueryLimits {
                    max_bytes: 1,
                    max_items: 0,
                },
                &Cancellation::default(),
            )
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(error.detail().len() <= 240);
        assert_eq!(d.unresolved_count(), 1);
    }
    let d = draft(BASE);
    for (records, enums, kind) in [
        (&["P", "P"][..], &[][..], ErrorKind::InvalidRequest),
        (&["missing"][..], &[][..], ErrorKind::UnknownTarget),
        (&[][..], &["E", "E"][..], ErrorKind::InvalidRequest),
        (&[][..], &["bad-name"][..], ErrorKind::InvalidRequest),
    ] {
        test(
            &d,
            SignatureSelection::Explicit {
                records,
                enums,
                callables: &[],
            },
            kind,
        );
    }
    for (callables, kind) in [
        (
            vec![BodyTarget::function("missing")],
            ErrorKind::UnknownTarget,
        ),
        (
            vec![BodyTarget::function("bad-name")],
            ErrorKind::InvalidRequest,
        ),
        (
            vec![BodyTarget::function("helper"); 2],
            ErrorKind::InvalidRequest,
        ),
        (
            vec![BodyTarget::function("helper"); 4097],
            ErrorKind::ResourceLimit,
        ),
    ] {
        test(
            &d,
            SignatureSelection::Explicit {
                records: &[],
                enums: &[],
                callables: &callables,
            },
            kind,
        );
    }
}

#[test]
fn explicit_receiver_identity_disambiguates_function_and_none_named_record() {
    let text = "fa1 v2 rec none 1 n int method none calc 1 self record none int ret field self n pub calc 0 int ret 5";
    let free = BodyTarget::function("calc");
    let method = BodyTarget::method("none", "calc");
    let cancel = Cancellation::default();
    let d = AuthoringDraft::open_fa1(text, 1, [2; 32], &[free.clone(), method.clone()], &cancel)
        .unwrap();
    assert_eq!(
        d.resolve_name("calc").unwrap_err().kind,
        ErrorKind::AmbiguousTarget
    );
    let callables = [method.clone()];
    let a = d
        .query_compact(
            d.binding(),
            &free,
            SignatureSelection::Explicit {
                records: &["none"],
                enums: &[],
                callables: &callables,
            },
            QueryLimits::default(),
            &cancel,
        )
        .unwrap();
    let b = d
        .query_compact(
            d.binding(),
            &method,
            none(),
            QueryLimits::default(),
            &cancel,
        )
        .unwrap();
    assert!(a.packet().contains("target function calc\n"));
    assert!(
        a.packet()
            .contains("method none calc 1 self record none int\n")
    );
    assert!(!a.packet().contains("fn calc "));
    assert!(b.packet().contains("target method none calc\n"));
    assert_eq!((a.selected_items(), a.total_items()), (2, 3));
}

#[test]
fn full_private_binding_and_generation_authority_survive_packet_projection() {
    let d = draft(BASE);
    let target = BodyTarget::function("calc");
    let c = compact(&d, none(), QueryLimits::default());
    let spaced = draft(&BASE.replace('\n', "\r\n  "));
    assert_eq!(
        d.binding().base_semantic_sha256,
        spaced.binding().base_semantic_sha256
    );
    assert_ne!(
        c.packet(),
        compact(&spaced, none(), QueryLimits::default()).packet()
    );
    assert_eq!(
        spaced
            .fill_body(&c, "fa1 body 1 ret x", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Stale
    );
    for field in 0..10 {
        let mut binding = d.binding().clone();
        match field {
            0 => binding.profile = "invented",
            1 => binding.compiler_contract = "invented",
            2 => binding.compiler_sha256[0] ^= 1,
            3 => binding.buffer_version += 1,
            4 => binding.generation += 1,
            5 => binding.source_sha256[0] ^= 1,
            6 => binding.base_semantic_sha256[0] ^= 1,
            7 => binding.node_index_sha256[0] ^= 1,
            8 => binding.sketch_sha256[0] ^= 1,
            9 => binding.draft_sha256[0] ^= 1,
            _ => unreachable!(),
        }
        assert_eq!(
            d.query_compact(
                &binding,
                &target,
                none(),
                QueryLimits::default(),
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::Stale
        );
    }
    let next = d
        .fill_body(&c, "fa1 body 1 ret x", &Cancellation::default())
        .unwrap();
    assert_eq!(
        next.fill_body(&c, "fa1 body 1 ret x", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Stale
    );
    assert_eq!(
        next.query_compact(
            next.binding(),
            &target,
            none(),
            QueryLimits::default(),
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::AlreadyFilled
    );
    assert_eq!(d.binding(), c.binding());
    assert_eq!(d.unresolved_count(), 1);
}

#[test]
fn oversized_optional_signature_is_omitted_whole_without_losing_counts() {
    let fields = (0..256)
        .map(|i| format!("f{i:03}{} int", "x".repeat(60)))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!("fa1 v2 rec Huge 256 {fields}\npub calc 0 int ret 0\n");
    let d = draft(&source);
    let selection = SignatureSelection::Explicit {
        records: &["Huge"],
        enums: &[],
        callables: &[],
    };
    let c = compact(
        &d,
        selection,
        QueryLimits {
            max_bytes: 600,
            max_items: 128,
        },
    );
    assert!(c.truncated());
    assert_eq!(
        (c.visible_items(), c.selected_items(), c.total_items()),
        (0, 1, 2)
    );
    assert!(c.packet().len() <= 600);
    assert!(!c.packet().contains("rec Huge"));
    let empty = compact(
        &d,
        none(),
        QueryLimits {
            max_bytes: 600,
            max_items: 0,
        },
    );
    assert!(!empty.truncated());
    assert_eq!(
        (
            empty.visible_items(),
            empty.selected_items(),
            empty.total_items()
        ),
        (0, 0, 2)
    );
    let full = compact(
        &d,
        selection,
        QueryLimits {
            max_bytes: 65536,
            max_items: 4096,
        },
    );
    assert!(!full.truncated());
    assert_eq!(full.visible_items(), 1);
    assert!(full.packet().contains("rec Huge 256 "));
}

#[test]
fn cancellation_and_absolute_request_limits_refuse_without_mutation() {
    let d = draft(BASE);
    let target = BodyTarget::function("calc");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        d.query_compact(
            d.binding(),
            &target,
            SignatureSelection::All,
            QueryLimits::default(),
            &cancel
        )
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
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
            max_bytes: 4096,
            max_items: 4097,
        },
    ] {
        assert_eq!(
            d.query_compact(
                d.binding(),
                &target,
                none(),
                limits,
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::InvalidRequest
        );
    }
    assert_eq!(d.unresolved_count(), 1);
}
