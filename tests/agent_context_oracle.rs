//! Independently authored AC1 oracles: literal ASTs, checked Rust arithmetic,
//! observable trap order, immutable tickets and bounded COW behavior.
//! These are new tests in this workspace, not recovered prior-session bytes.
use fluxr::structured::authoring::{
    AuthoringDraft, BodyTarget, COMPACT_CONTEXT_PROFILE, Cancellation, ErrorKind, QueryLimits,
    SignatureSelection,
};
use fluxr::structured::{
    Binary, Binding, Expr, Function, Invocation, Limits, Module, Record, Source, Stmt, Type, Value,
    Workspace,
};
use std::sync::Arc;

const BASE: &str = "fa1 v2\npub calc 2 x int y int int ret div x y\n";

fn binding(name: &str, ty: Type) -> Binding {
    Binding {
        name: name.into(),
        ty,
    }
}

fn local(name: &str) -> Expr {
    Expr::Local { name: name.into() }
}

fn integer(value: i64) -> Expr {
    Expr::I64 { value }
}

fn binary(kind: Binary, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn function(name: &str, params: Vec<Binding>, result: Type, body: Vec<Stmt>) -> Function {
    Function {
        name: name.into(),
        receiver: None,
        export: true,
        params,
        locals: vec![],
        result,
        body,
    }
}

fn source(functions: Vec<Function>) -> Source {
    Source {
        version: 2,
        inputs: vec![],
        records: vec![],
        enums: vec![],
        functions,
    }
}

fn draft(text: &str, targets: &[BodyTarget]) -> AuthoringDraft {
    AuthoringDraft::open_fa1(text, 17, [23; 32], targets, &Cancellation::default()).unwrap()
}

fn completed(text: &str, name: &str, fill: &str) -> Module {
    let target = BodyTarget::function(name);
    let d = draft(text, std::slice::from_ref(&target));
    let context = d
        .query_compact(
            d.binding(),
            &target,
            SignatureSelection::Explicit {
                records: &[],
                enums: &[],
                callables: &[],
            },
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let next = d
        .fill_body(&context, fill, &Cancellation::default())
        .unwrap();
    next.admit(next.binding(), &Cancellation::default())
        .unwrap()
}

fn run(
    module: &Module,
    arguments: &[Value],
    inputs: &[Value],
    limits: Limits,
) -> Result<Value, String> {
    module
        .run(Invocation {
            export: "calc",
            arguments,
            inputs,
            limits,
        })
        .map_err(|e| e.to_string())
}

#[test]
fn oracle_compact_scope_revision_and_uncertainty_remain_visible() {
    let text = "fa1 v2 inputs 1 gain int pub calc 2 x int y int int do 1 scratch int 1 ret add x y fn helper 1 x int int ret x";
    let target = BodyTarget::function("calc");
    let d = draft(text, std::slice::from_ref(&target));
    let c = d
        .query_compact(
            d.binding(),
            &target,
            SignatureSelection::Explicit {
                records: &[],
                enums: &[],
                callables: &[],
            },
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(c.packet_profile(), COMPACT_CONTEXT_PROFILE);
    assert_eq!(c.binding(), d.binding());
    assert_eq!(c.target(), &target);
    assert_eq!(c.expected_type(), &Type::I64);
    assert!(c.analysis_only());
    assert!(!c.truncated());
    assert_eq!(
        (
            c.visible_items(),
            c.selected_items(),
            c.total_items(),
            c.unselected_items()
        ),
        (4, 4, 6, 2)
    );
    let digest: String = d
        .binding()
        .draft_sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    for required in [
        "ac1 analysis_only\n",
        "target function calc\n",
        "expected int\n",
        "effects unproven\n",
        "resources unproven\n",
        "capabilities unproven\n",
        "requires whole_module_check invocation_limits\n",
        "param x int\n",
        "param y int\n",
        "local scratch int\n",
        "input gain int\n",
        "admission none\n",
    ] {
        assert!(c.packet().contains(required), "missing {required:?}");
    }
    assert!(c.packet().contains(&format!("draft {digest}\n")));
    assert!(!c.packet().contains("fn helper"));
    assert!(!c.packet().contains("ret add"));
    assert_eq!(
        d.admit(d.binding(), &Cancellation::default())
            .unwrap_err()
            .kind,
        ErrorKind::Unresolved
    );
    assert!(
        d.fill_body(&c, c.packet(), &Cancellation::default())
            .is_err()
    );
    assert_eq!(d.unresolved_count(), 1);
    let spaced = draft(&text.replace(' ', "  "), std::slice::from_ref(&target));
    let other = spaced
        .query_compact(
            spaced.binding(),
            &target,
            SignatureSelection::All,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_ne!(d.binding().draft_sha256, spaced.binding().draft_sha256);
    assert_ne!(c.packet(), other.packet());
    assert_eq!(
        d.binding().base_semantic_sha256,
        spaced.binding().base_semantic_sha256
    );
}

#[test]
fn oracle_selectors_validate_before_truncation_and_keep_source_order() {
    let text = "fa1 v2 rec Z 1 n int rec A 1 n int enum E 1 tag none fn z 0 int ret 1 pub calc 0 int ret 0 fn a 0 int ret 2";
    let target = BodyTarget::function("calc");
    let d = draft(text, std::slice::from_ref(&target));
    let names = [BodyTarget::function("a"), BodyTarget::function("z")];
    let c = d
        .query_compact(
            d.binding(),
            &target,
            SignatureSelection::Explicit {
                records: &["A", "Z"],
                enums: &["E"],
                callables: &names,
            },
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(c.packet().find("rec Z").unwrap() < c.packet().find("rec A").unwrap());
    assert!(c.packet().find("fn z").unwrap() < c.packet().find("fn a").unwrap());
    assert_eq!(
        (c.visible_items(), c.selected_items(), c.total_items()),
        (5, 5, 6)
    );
    for (names, kind) in [
        (
            vec![BodyTarget::function("missing")],
            ErrorKind::UnknownTarget,
        ),
        (
            vec![BodyTarget::function("bad-name")],
            ErrorKind::InvalidRequest,
        ),
        (
            vec![BodyTarget::function("z"), BodyTarget::function("z")],
            ErrorKind::InvalidRequest,
        ),
    ] {
        let e = d
            .query_compact(
                d.binding(),
                &target,
                SignatureSelection::Explicit {
                    records: &[],
                    enums: &[],
                    callables: &names,
                },
                QueryLimits {
                    max_bytes: 1,
                    max_items: 0,
                },
                &Cancellation::default(),
            )
            .unwrap_err();
        assert_eq!(e.kind, kind);
    }
    let omitted = d
        .query_compact(
            d.binding(),
            &target,
            SignatureSelection::All,
            QueryLimits {
                max_bytes: 2048,
                max_items: 0,
            },
            &Cancellation::default(),
        )
        .unwrap();
    assert!(omitted.truncated());
    assert_eq!(
        (
            omitted.visible_items(),
            omitted.selected_items(),
            omitted.total_items()
        ),
        (0, 6, 6)
    );
}

#[test]
fn oracle_full_sidecar_staleness_and_failed_fill_preserve_immutable_forks() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let c = d
        .query_compact(
            d.binding(),
            &target,
            SignatureSelection::All,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let original = d.binding().clone();
    for index in 0..10 {
        let mut wrong = original.clone();
        match index {
            0 => wrong.profile = "unrecognized",
            1 => wrong.compiler_contract = "unrecognized",
            2 => wrong.compiler_sha256[0] ^= 1,
            3 => wrong.buffer_version += 1,
            4 => wrong.generation += 1,
            5 => wrong.source_sha256[0] ^= 1,
            6 => wrong.base_semantic_sha256[0] ^= 1,
            7 => wrong.node_index_sha256[0] ^= 1,
            8 => wrong.sketch_sha256[0] ^= 1,
            _ => wrong.draft_sha256[0] ^= 1,
        }
        assert_eq!(
            d.query_compact(
                &wrong,
                &target,
                SignatureSelection::All,
                QueryLimits::default(),
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::Stale
        );
    }
    assert_eq!(
        d.fill_body(&c, "fa1 body 1 ret true", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(d.binding(), &original);
    let left = d
        .fill_body(&c, "fa1 body 1 ret add x y", &Cancellation::default())
        .unwrap();
    let right = d
        .fill_body(&c, "fa1 body 1 ret sub x y", &Cancellation::default())
        .unwrap();
    assert_ne!(left.binding().draft_sha256, right.binding().draft_sha256);
    assert_eq!(
        left.fill_body(&c, "fa1 body 1 ret x", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Stale
    );
    let other_compiler =
        AuthoringDraft::open_fa1(BASE, 17, [24; 32], &[target], &Cancellation::default()).unwrap();
    assert_eq!(
        other_compiler
            .fill_body(&c, "fa1 body 1 ret x", &Cancellation::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::Stale
    );
    assert_eq!(d.unresolved_count(), 1);
    assert_eq!(d.binding(), &original);
}

#[test]
fn oracle_mandatory_scope_caps_and_terminal_cancellation_cannot_admit() {
    let target = BodyTarget::function("calc");
    let d = draft(BASE, std::slice::from_ref(&target));
    let query = |limits, cancel: &Cancellation| {
        d.query_compact(
            d.binding(),
            &target,
            SignatureSelection::Explicit {
                records: &[],
                enums: &[],
                callables: &[],
            },
            limits,
            cancel,
        )
    };
    for bytes in [1, 64, 128] {
        assert_eq!(
            query(
                QueryLimits {
                    max_bytes: bytes,
                    max_items: 2
                },
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::OutputLimit
        );
    }
    assert_eq!(
        query(
            QueryLimits {
                max_bytes: 2048,
                max_items: 1
            },
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        ErrorKind::OutputLimit
    );
    for limits in [
        QueryLimits {
            max_bytes: 0,
            max_items: 2,
        },
        QueryLimits {
            max_bytes: 65537,
            max_items: 2,
        },
        QueryLimits {
            max_bytes: 2048,
            max_items: 4097,
        },
    ] {
        assert_eq!(
            query(limits, &Cancellation::default()).unwrap_err().kind,
            ErrorKind::InvalidRequest
        );
    }
    let c = query(QueryLimits::default(), &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let alias = cancel.clone();
    cancel.cancel();
    assert!(alias.is_cancelled());
    assert_eq!(
        query(QueryLimits::default(), &alias).unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        d.fill_body(&c, "fa1 body 1 ret x", &alias)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        d.admit(d.binding(), &alias).unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert!(query(QueryLimits::default(), &Cancellation::default()).is_ok());
}

#[test]
fn oracle_scalar_ast_checked_rust_and_all_stated_budget_outcomes() {
    let candidate = completed(BASE, "calc", "fa1 body 1 ret div add x 1 y");
    let independent = Module::compile(source(vec![function(
        "calc",
        vec![binding("x", Type::I64), binding("y", Type::I64)],
        Type::I64,
        vec![Stmt::Return {
            value: binary(
                Binary::Div,
                binary(Binary::Add, local("x"), integer(1)),
                local("y"),
            ),
        }],
    )]))
    .unwrap();
    assert_eq!(candidate.encode(), independent.encode());
    let mut pairs = 0;
    let mut rust_cases = 0;
    for x in [i64::MIN, i64::MIN + 1, -2, -1, 0, 7, i64::MAX] {
        for y in [-1, 0, 1, 3] {
            let args = [Value::I64(x), Value::I64(y)];
            let expected = x
                .checked_add(1)
                .and_then(|x| x.checked_div(y))
                .map(Value::I64)
                .ok_or_else(|| "invalid or overflowing integer arithmetic".to_owned());
            assert_eq!(run(&candidate, &args, &[], Limits::default()), expected);
            rust_cases += 1;
            for instructions in 0..20 {
                for cells in 0..12 {
                    for frames in 0..3 {
                        let limits = Limits {
                            instructions,
                            cells,
                            frames,
                        };
                        assert_eq!(
                            run(&candidate, &args, &[], limits),
                            run(&independent, &args, &[], limits)
                        );
                        pairs += 1;
                    }
                }
            }
        }
    }
    assert_eq!((pairs, rust_cases), (20160, 28));
    println!(
        "AC1_ORACLE scalar_budget_pairs={pairs} scalar_budget_vm_calls={} checked_rust_cases={rust_cases}",
        pairs * 2
    );
}

#[test]
fn oracle_distinct_first_traps_follow_source_evaluation_order() {
    let base = "fa1 v2 pub calc 1 xs list int int ret 0";
    let index = || Expr::Index {
        list: Box::new(local("xs")),
        index: Box::new(Expr::U64 { value: 0 }),
    };
    let divide = || binary(Binary::Div, integer(1), integer(0));
    let mut cases = 0;
    for reversed in [false, true] {
        let fill = if reversed {
            "fa1 body 1 ret add div 1 0 at xs 0u"
        } else {
            "fa1 body 1 ret add at xs 0u div 1 0"
        };
        let candidate = completed(base, "calc", fill);
        let expr = if reversed {
            binary(Binary::Add, divide(), index())
        } else {
            binary(Binary::Add, index(), divide())
        };
        let independent = Module::compile(source(vec![function(
            "calc",
            vec![binding("xs", Type::List(Box::new(Type::I64)))],
            Type::I64,
            vec![Stmt::Return { value: expr }],
        )]))
        .unwrap();
        assert_eq!(candidate.encode(), independent.encode());
        for values in [vec![], vec![Value::I64(9)]] {
            let expected = if !reversed && values.is_empty() {
                "list index out of bounds"
            } else {
                "invalid or overflowing integer arithmetic"
            };
            let args = [Value::List(Arc::new(values))];
            let result = run(&candidate, &args, &[], Limits::default());
            assert_eq!(result, Err(expected.to_owned()));
            assert_eq!(result, run(&independent, &args, &[], Limits::default()));
            cases += 1;
        }
    }
    assert_eq!(cases, 4);
    println!("AC1_ORACLE distinct_first_trap_cases={cases}");
}

#[test]
fn oracle_nested_cow_input_snapshot_and_cells_match_independent_ast() {
    let text = "fa1 v2 inputs 1 snapshot list int rec Box 2 xs list int n int pub calc 1 b record Box record Box do 1 work list int 1 ret b";
    let candidate = completed(
        text,
        "calc",
        "fa1 body 4 set work field b xs pop work put b xs work ret b",
    );
    let list = Type::List(Box::new(Type::I64));
    let mut f = function(
        "calc",
        vec![binding("b", Type::Record("Box".into()))],
        Type::Record("Box".into()),
        vec![
            Stmt::Set {
                local: "work".into(),
                value: Expr::Field {
                    record: Box::new(local("b")),
                    name: "xs".into(),
                },
            },
            Stmt::Pop {
                local: "work".into(),
            },
            Stmt::SetField {
                local: "b".into(),
                field: "xs".into(),
                value: local("work"),
            },
            Stmt::Return { value: local("b") },
        ],
    );
    f.locals = vec![binding("work", list.clone())];
    let mut s = source(vec![f]);
    s.inputs = vec![binding("snapshot", list.clone())];
    s.records = vec![Record {
        name: "Box".into(),
        fields: vec![binding("xs", list), binding("n", Type::I64)],
    }];
    let independent = Module::compile(s).unwrap();
    assert_eq!(candidate.encode(), independent.encode());
    let original = Arc::new(vec![Value::I64(7), Value::I64(8)]);
    let record: Arc<[Value]> = Arc::from([Value::List(Arc::clone(&original)), Value::I64(9)]);
    let args = [Value::Record(Arc::clone(&record))];
    let inputs = [Value::List(Arc::clone(&original))];
    let expected = Value::Record(Arc::from([
        Value::List(Arc::new(vec![Value::I64(7)])),
        Value::I64(9),
    ]));
    assert_eq!(
        run(&candidate, &args, &inputs, Limits::default()),
        Ok(expected)
    );
    let (mut a, mut b) = (Workspace::default(), Workspace::default());
    let mut pairs = 0;
    let mut failures = 0;
    let mut successes = 0;
    for cells in 0..32 {
        let invocation = || Invocation {
            export: "calc",
            arguments: &args,
            inputs: &inputs,
            limits: Limits {
                cells,
                ..Limits::default()
            },
        };
        let result = candidate
            .run_with(invocation(), &mut a)
            .map_err(|e| e.to_string());
        assert_eq!(
            result,
            independent
                .run_with(invocation(), &mut b)
                .map_err(|e| e.to_string())
        );
        if result.is_ok() {
            successes += 1;
        } else {
            failures += 1;
        }
        assert_eq!(*original, vec![Value::I64(7), Value::I64(8)]);
        assert_eq!(
            record.as_ref(),
            &[Value::List(Arc::clone(&original)), Value::I64(9)]
        );
        pairs += 1;
    }
    assert!(successes > 0 && failures > 0);
    assert_eq!(pairs, 32);
    println!(
        "AC1_ORACLE cow_budget_pairs={pairs} cow_successes={successes} cow_refusals={failures}"
    );
}

#[test]
fn oracle_raw_float_literal_bits_survive_fill_and_fsm_encoding() {
    let patterns: [u32; 12] = [
        0, 0x80000000, 0x3f800000, 0xbf800000, 1, 0x007fffff, 0x00800000, 0x7f7fffff, 0x7f800000,
        0xff800000, 0x7fc01234, 0x7fa01234,
    ];
    for bits in patterns {
        let candidate = completed(
            "fa1 v2 pub calc 0 float ret 0x00000000f",
            "calc",
            &format!("fa1 body 1 ret 0x{bits:08x}f"),
        );
        let independent = Module::compile(source(vec![function(
            "calc",
            vec![],
            Type::F32,
            vec![Stmt::Return {
                value: Expr::F32 { bits },
            }],
        )]))
        .unwrap();
        assert_eq!(candidate.encode(), independent.encode());
        assert_eq!(
            run(&candidate, &[], &[], Limits::default()),
            Ok(Value::F32(bits))
        );
    }
    println!("AC1_ORACLE raw_f32_patterns={}", patterns.len());
}

#[test]
fn oracle_literal_none_receiver_never_collides_with_free_function() {
    let text = "fa1 v2 rec none 1 n int method none calc 1 self record none int ret field self n pub calc 0 int ret 5";
    let free = BodyTarget::function("calc");
    let method = BodyTarget::method("none", "calc");
    let d = draft(text, &[free.clone(), method.clone()]);
    assert_eq!(
        d.resolve_name("calc").unwrap_err().kind,
        ErrorKind::AmbiguousTarget
    );
    let c = d
        .query_compact(
            d.binding(),
            &free,
            SignatureSelection::Explicit {
                records: &[],
                enums: &[],
                callables: std::slice::from_ref(&method),
            },
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(c.packet().contains("target function calc\n"));
    assert!(c.packet().contains("method none calc"));
    let first = d
        .fill_body(
            &c,
            "fa1 body 1 ret m new none 1 7 calc 0",
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        first
            .admit(first.binding(), &Cancellation::default())
            .unwrap_err()
            .kind,
        ErrorKind::Unresolved
    );
    let m = first
        .query_compact(
            first.binding(),
            &method,
            SignatureSelection::All,
            QueryLimits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(m.packet().contains("target method none calc\n"));
    let ready = first
        .fill_body(&m, "fa1 body 1 ret field self n", &Cancellation::default())
        .unwrap();
    let candidate = ready
        .admit(ready.binding(), &Cancellation::default())
        .unwrap();
    let mut method_ast = function(
        "calc",
        vec![binding("self", Type::Record("none".into()))],
        Type::I64,
        vec![Stmt::Return {
            value: Expr::Field {
                record: Box::new(local("self")),
                name: "n".into(),
            },
        }],
    );
    method_ast.receiver = Some("none".into());
    method_ast.export = false;
    let free_ast = function(
        "calc",
        vec![],
        Type::I64,
        vec![Stmt::Return {
            value: Expr::Method {
                receiver: Box::new(Expr::Record {
                    name: "none".into(),
                    fields: vec![integer(7)],
                }),
                name: "calc".into(),
                args: vec![],
            },
        }],
    );
    let mut s = source(vec![method_ast, free_ast]);
    s.records = vec![Record {
        name: "none".into(),
        fields: vec![binding("n", Type::I64)],
    }];
    let independent = Module::compile(s).unwrap();
    assert_eq!(candidate.encode(), independent.encode());
    assert_eq!(
        run(&candidate, &[], &[], Limits::default()),
        Ok(Value::I64(7))
    );
    println!("AC1_ORACLE method_free_name_collision_cases=1");
}
