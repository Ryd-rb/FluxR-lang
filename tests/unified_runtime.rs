//! Behavioral tests for the one source loader and process runtime. Native
//! service callbacks below are independent Rust oracles with visible effects.
use fluxr::runtime::{
    self, Command, ControlRequest, ControlResponse, ControlService, Document, EmbeddedSource,
    ImportKind, Limits, Runtime, RuntimeConfig, RuntimeError, Type, Value,
};
use fluxr::structured::{Invocation, Module, Workspace};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const PURE_DIGEST: &str = "184bd7091a452cdfd323bc0e6916f1a01bd2753aa087485dc5429a73896a12de";
fn source(value: u64) -> String {
    format!("fluxr 1\npub value 0 uint ret {value}u")
}
fn call(runtime: &Runtime, name: &str) -> runtime::CallResult {
    runtime
        .call(name, "value", &[], &[], Limits::default())
        .unwrap()
}
fn leaked_source(name: &'static str, source: String) -> EmbeddedSource {
    let digest = Document::parse(&source).unwrap().native_digest;
    EmbeddedSource {
        name,
        source: Box::leak(source.into_boxed_str()),
        native_digest: Box::leak(digest.into_boxed_str()),
    }
}

#[test]
fn one_header_and_exact_native_spans_match_the_independent_frontend_corpus() {
    #[derive(Deserialize)]
    struct Case {
        name: String,
        source: String,
        native_digest: String,
        native_spans: Vec<(usize, usize)>,
        checked: String,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../tools/unified_tests/native_spans.json")).unwrap();
    for case in cases {
        let document =
            Document::parse(&case.source).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(document.native_digest, case.native_digest, "{}", case.name);
        assert_eq!(
            document
                .native_spans
                .iter()
                .map(|s| (s.start, s.end))
                .collect::<Vec<_>>(),
            case.native_spans,
            "{}",
            case.name
        );
        assert_eq!(
            document.checked_text,
            format!("fluxr 1\n{}", case.checked),
            "{}",
            case.name
        );
        let module = document
            .compile()
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(
            Module::decode(&module.encode()).unwrap().encode(),
            module.encode()
        );
    }
    for invalid in [
        "fa1 v2 pub value 0 int ret 1",
        "fluxr 2\n",
        "fluxr 1\r\n",
        "fluxr 1 ",
        "fluxr 1\nunknown",
        "fluxr 1\nrust 3\né",
        "fluxr 1\nrust 1 x",
        "fluxr 1\nnative-fn f 999999999 unit 0",
        "fluxr 1\nnative-fn f 0 unit 1 tail str \"unterminated",
        "fluxr 1\nnative-fn f 0 unit 1 tail str \"\\ud800\"",
    ] {
        assert!(Document::parse(invalid).is_err(), "accepted {invalid:?}");
    }
    let empty = Document::parse("fluxr 1\n").unwrap().compile().unwrap();
    assert!(empty.export_signatures().is_empty());
    assert_eq!(empty.unified_text().unwrap(), "fluxr 1\n");
}

#[test]
fn historical_encodings_import_to_one_language_and_preserve_behavior() {
    for text in [
        "fa1 v1 pub value 0 int ret 41",
        "fa1 v2 pub value 0 int ret 41",
    ] {
        let canonical = runtime::import_source(ImportKind::Agent, text.as_bytes()).unwrap();
        assert!(canonical.starts_with("fluxr 1\n"));
        let runtime = Runtime::new();
        runtime.install("module", None, &canonical).unwrap();
        assert_eq!(call(&runtime, "module").value, Value::I64(41));
    }
    let compact = "v2 (pub value () i64 = 41)";
    let module = Module::compile_compact(compact).unwrap();
    let fsm = runtime::import_source(ImportKind::Fsm, &module.encode()).unwrap();
    assert_eq!(
        fsm,
        runtime::import_source(ImportKind::Compact, compact.as_bytes()).unwrap()
    );
    let dictionary = fluxr::structured::wordcode::Dictionary::core();
    let wk = format!("wk1 {} v2 pub value 0 int ret 41", dictionary.header());
    assert_eq!(
        runtime::import_source(ImportKind::Wordcode, wk.as_bytes()).unwrap(),
        fsm
    );
    let with_host_local =
        Module::compile_agent("fa1 v2 pub value 1 host int int ret host").unwrap();
    assert!(
        with_host_local
            .unified_text()
            .unwrap()
            .ends_with("ret local host")
    );
    assert_eq!(
        Document::parse(&with_host_local.unified_text().unwrap())
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        with_host_local.encode()
    );
}

#[test]
fn checked_admission_and_rejected_updates_do_not_publish_or_execute() {
    let runtime = Runtime::new();
    assert_eq!(
        runtime
            .install("counter", None, &source(1))
            .unwrap()
            .revision,
        1
    );
    let before = runtime.inspect("counter").unwrap();
    for (expected, text) in [
        (None, source(2)),
        (Some(0), source(2)),
        (Some(1), "fluxr 1\npub value 0 bool ret true".into()),
        (Some(1), "fluxr 1\npub value 0 uint ret true".into()),
        (
            Some(1),
            "fluxr 1\npub value 0 uint ret host unavailable uint 0".into(),
        ),
    ] {
        assert!(runtime.install("counter", expected, &text).is_err());
        let after = runtime.inspect("counter").unwrap();
        assert_eq!(
            (after.revision, after.source_sha256),
            (before.revision, before.source_sha256.clone())
        );
        assert_eq!(call(&runtime, "counter").value, Value::U64(1));
    }
    assert_eq!(
        runtime
            .install("counter", Some(1), &source(2))
            .unwrap()
            .revision,
        2
    );
    assert!(runtime.remove("counter", 1).is_err());
    runtime.remove("counter", 2).unwrap();
    assert_eq!(
        runtime
            .install("counter", None, &source(3))
            .unwrap()
            .revision,
        3
    );
    assert!(runtime.install("counter", Some(1), &source(4)).is_err());
}

#[test]
fn snapshots_keep_old_revisions_and_concurrent_readers_never_mix_results() {
    let runtime = Arc::new(Runtime::new());
    runtime.install("hot", None, &source(0)).unwrap();
    let old = runtime.snapshot("hot").unwrap();
    let prepared = old.prepare_export("value").unwrap();
    let go = Arc::new(Barrier::new(5));
    let done = Arc::new(AtomicBool::new(false));
    thread::scope(|scope| {
        for _ in 0..4 {
            let (runtime, go, done) = (Arc::clone(&runtime), Arc::clone(&go), Arc::clone(&done));
            scope.spawn(move || {
                go.wait();
                let mut reads = 0;
                while reads < 500 || !done.load(Ordering::Acquire) {
                    let result = call(&runtime, "hot");
                    assert_eq!(result.value, Value::U64(result.revision - 1));
                    reads += 1;
                }
            });
        }
        go.wait();
        for revision in 1..=80 {
            assert_eq!(
                runtime
                    .install("hot", Some(revision), &source(revision))
                    .unwrap()
                    .revision,
                revision + 1
            );
        }
        done.store(true, Ordering::Release);
    });
    assert_eq!(
        old.call("value", &[], &[], Limits::default())
            .unwrap()
            .value,
        Value::U64(0)
    );
    assert_eq!(old.revision(), 1);
    let mut workspace = Workspace::default();
    assert_eq!(
        prepared
            .call_with(&[], &[], Limits::default(), &mut workspace)
            .unwrap()
            .value,
        Value::U64(0)
    );
    assert_eq!(prepared.revision(), 1);
    runtime.remove("hot", 81).unwrap();
    assert_eq!(
        old.call("value", &[], &[], Limits::default())
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn an_inflight_host_call_retains_its_revision_without_holding_publication_locks() {
    let runtime = Arc::new(Runtime::new());
    let release = Arc::new(Barrier::new(2));
    let (entered, wait) = mpsc::channel();
    let hold = Arc::clone(&release);
    runtime
        .register_host("pause", vec![], Type::U64, move |_| {
            entered.send(()).unwrap();
            hold.wait();
            Ok(Value::U64(7))
        })
        .unwrap();
    runtime
        .install(
            "hot",
            None,
            "fluxr 1\npub value 0 uint ret host pause uint 0",
        )
        .unwrap();
    let active = Arc::clone(&runtime);
    let worker = thread::spawn(move || call(&active, "hot"));
    wait.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        runtime
            .install("hot", Some(1), &source(9))
            .unwrap()
            .revision,
        2
    );
    assert_eq!(call(&runtime, "hot").value, Value::U64(9));
    release.wait();
    let result = worker.join().unwrap();
    assert_eq!((result.revision, result.value), (1, Value::U64(7)));
}

#[test]
fn host_calls_are_typed_granted_reentrant_and_not_replayed_by_native_fallback() {
    let runtime = Arc::new(Runtime::new());
    let count = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&count);
    runtime
        .register_host("bump", vec![Type::U64], Type::U64, move |arguments| {
            let [Value::U64(delta)] = arguments else {
                panic!("unchecked host arguments")
            };
            Ok(Value::U64(
                counter.fetch_add(*delta, Ordering::SeqCst) + delta,
            ))
        })
        .unwrap();
    assert!(
        runtime
            .register_host("bump", vec![], Type::U64, |_| Ok(Value::U64(0)))
            .is_err()
    );
    let text = "fluxr 1\nfn invoke 1 x uint uint ret host bump uint 1 uint x\npub value 0 uint ret call invoke 1 3u\npub pure 0 int ret 6";
    runtime.install("effects", None, text).unwrap();
    let module = runtime.snapshot("effects").unwrap();
    assert!(module.module().emit_native_rust("value").is_err());
    assert!(module.module().emit_native_rust("pure").is_ok());
    assert!(!module.module().prepare_native("value"));
    assert_eq!(call(&runtime, "effects").value, Value::U64(3));
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(call(&runtime, "effects").route, "vm");
    assert_eq!(count.load(Ordering::SeqCst), 6);
    assert!(
        runtime
            .install(
                "wrong",
                None,
                "fluxr 1\npub value 0 int ret host bump int 1 uint 1u"
            )
            .is_err()
    );
    assert!(
        Document::parse("fluxr 1\npub value 0 uint ret host bump uint 1 uint true")
            .unwrap()
            .compile()
            .is_err()
    );
    assert!(Document::parse("fluxr 1\nfn a 0 uint ret host bump uint 1 uint 1u\npub value 0 int ret host bump int 1 uint 1u").unwrap().compile().is_err());
    let weak = Arc::downgrade(&runtime);
    runtime
        .register_host("nested", vec![], Type::U64, move |_| {
            let runtime = weak.upgrade().unwrap();
            runtime.install("nested_result", None, &source(42))?;
            Ok(call(&runtime, "nested_result").value)
        })
        .unwrap();
    runtime
        .install(
            "outer",
            None,
            "fluxr 1\npub value 0 uint ret host nested uint 0",
        )
        .unwrap();
    let outer = runtime.snapshot("outer").unwrap();
    let prepared = outer.prepare_export("value").unwrap();
    let result = prepared
        .call_with(&[], &[], Limits::default(), &mut Workspace::default())
        .unwrap();
    assert_eq!(result.value, Value::U64(42));
    assert_eq!(result.route, runtime::ExecutionRoute::Portable);
}

#[test]
fn host_results_errors_panics_and_object_budgets_remain_observable() {
    let runtime = Runtime::new();
    runtime
        .register_host("wrong", vec![], Type::U64, |_| Ok(Value::Bool(true)))
        .unwrap();
    runtime
        .register_host("fails", vec![], Type::U64, |_| {
            Err(RuntimeError::new("database", "transaction rolled back"))
        })
        .unwrap();
    runtime
        .register_host("panics", vec![], Type::U64, |_| {
            panic!("intentional callback panic")
        })
        .unwrap();
    for (host, fragment) in [
        ("wrong", "host value differs"),
        ("fails", "transaction rolled back"),
        ("panics", "host_panic"),
    ] {
        runtime
            .install(
                host,
                None,
                &format!("fluxr 1\npub value 0 uint ret host {host} uint 0"),
            )
            .unwrap();
        let error = runtime
            .call(host, "value", &[], &[], Limits::default())
            .unwrap_err();
        assert!(error.message.contains(fragment), "{error}");
        assert_eq!(runtime.inspect(host).unwrap().revision, 1);
    }
    let kind = Type::List(Box::new(Type::F32));
    runtime
        .register_host("echo", vec![kind.clone()], kind, |arguments| {
            Ok(arguments[0].clone())
        })
        .unwrap();
    runtime.install("float_bits", None, "fluxr 1\npub value 0 list float ret host echo list float 1 list float list float 3 0x7f800001f 0x80000000f 0xffc01234f").unwrap();
    assert_eq!(
        call(&runtime, "float_bits").value,
        Value::List(Arc::new(vec![
            Value::F32(0x7f800001),
            Value::F32(0x80000000),
            Value::F32(0xffc01234)
        ]))
    );
    assert!(
        runtime
            .call(
                "float_bits",
                "value",
                &[],
                &[],
                Limits {
                    cells: 2,
                    ..Limits::default()
                }
            )
            .is_err()
    );
    for ty in [
        Type::Record("Account".into()),
        Type::Enum("Result".into()),
        Type::List(Box::new(Type::Record("Account".into()))),
    ] {
        assert_eq!(
            runtime
                .register_host("nominal", vec![ty], Type::Unit, |_| Ok(Value::Unit))
                .unwrap_err()
                .code,
            "host_signature"
        );
    }
    runtime.install("healthy", None, &source(99)).unwrap();
    assert_eq!(call(&runtime, "healthy").value, Value::U64(99));
}

#[test]
fn native_extensions_require_exact_build_identity_and_checked_updates_use_same_loader() {
    let runtime = Runtime::new();
    let native = "native-fn foreign 0 usize 1 tail int 1usize";
    let original = format!("fluxr 1\n{native}\npub value 0 uint ret 7u");
    assert_eq!(
        runtime.install("mixed", None, &original).unwrap_err().code,
        "native_unbound"
    );
    let embedded = leaked_source("mixed", original.clone());
    runtime.install_embedded(embedded).unwrap();
    let next = original.replace("ret 7u", "ret 8u");
    runtime.install("mixed", Some(1), &next).unwrap();
    assert_eq!(call(&runtime, "mixed").value, Value::U64(8));
    for changed in [next.replace("1usize", "2usize"), source(9)] {
        assert_eq!(
            runtime
                .install("mixed", Some(2), &changed)
                .unwrap_err()
                .code,
            "native_identity"
        );
        assert_eq!(call(&runtime, "mixed").value, Value::U64(8));
    }
    assert_eq!(runtime.install_embedded(embedded).unwrap().revision, 2);
    assert_eq!(
        runtime
            .install_embedded(EmbeddedSource {
                name: "bad",
                source: embedded.source,
                native_digest: PURE_DIGEST
            })
            .unwrap_err()
            .code,
        "native_identity"
    );
}

#[test]
fn embedded_catalog_admission_is_atomic_and_host_binding_can_follow_bootstrap() {
    let runtime = Runtime::new();
    let first = leaked_source("first", source(1));
    let bad = leaked_source(
        "bad",
        "fluxr 1\npub value 0 uint ret host future uint 0".into(),
    );
    runtime
        .register_host("future", vec![], Type::Bool, |_| Ok(Value::Bool(true)))
        .unwrap();
    assert!(runtime.install_embedded_sources(&[first, bad]).is_err());
    assert!(runtime.list().unwrap().is_empty());
    assert!(runtime.install_embedded_sources(&[first, first]).is_err());
    assert!(runtime.list().unwrap().is_empty());
    let runtime = Runtime::new();
    runtime.install_embedded_sources(&[first, bad]).unwrap();
    assert!(
        runtime
            .call("bad", "value", &[], &[], Limits::default())
            .unwrap_err()
            .message
            .contains("host_missing")
    );
    assert!(
        runtime
            .register_host("future", vec![], Type::Bool, |_| Ok(Value::Bool(true)))
            .is_err()
    );
    runtime
        .register_host("future", vec![], Type::U64, |_| Ok(Value::U64(19)))
        .unwrap();
    assert_eq!(call(&runtime, "bad").value, Value::U64(19));
    runtime.install("first", Some(1), &source(2)).unwrap();
    assert_eq!(
        runtime.install_embedded_sources(&[first, bad]).unwrap()[0].revision,
        2
    );
    let incompatible = leaked_source(
        "conflicting",
        "fluxr 1\npub value 0 bool ret host unbound bool 0".into(),
    );
    let unsigned = leaked_source(
        "unsigned",
        "fluxr 1\npub value 0 uint ret host unbound uint 0".into(),
    );
    assert!(
        runtime
            .install_embedded_sources(&[incompatible, unsigned])
            .is_err()
    );
    assert_eq!(runtime.list().unwrap().len(), 2);
}

#[test]
fn capacities_and_per_call_input_values_do_not_expand_host_authority() {
    let runtime = Runtime::with_config(RuntimeConfig {
        max_modules: 1,
        max_names: 2,
        max_source_bytes: 512,
        max_active_bytes: 512,
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime.install("a", None, &source(1)).unwrap();
    assert_eq!(
        runtime.install("b", None, &source(2)).unwrap_err().code,
        "capacity"
    );
    runtime.remove("a", 1).unwrap();
    runtime.install("b", None, &source(2)).unwrap();
    runtime.remove("b", 1).unwrap();
    assert_eq!(
        runtime.install("c", None, &source(3)).unwrap_err().code,
        "capacity"
    );
    runtime
        .install(
            "a",
            None,
            "fluxr 1\ninputs 1 factor uint\npub value 1 n uint uint ret mul input factor n",
        )
        .unwrap();
    assert_eq!(
        runtime
            .call(
                "a",
                "value",
                &[Value::U64(7)],
                &[Value::U64(6)],
                Limits::default()
            )
            .unwrap()
            .value,
        Value::U64(42)
    );
    assert!(
        runtime
            .call("a", "value", &[], &[Value::U64(6)], Limits::default())
            .is_err()
    );
    assert_eq!(
        runtime
            .call(
                "a",
                "value",
                &[],
                &[],
                Limits {
                    instructions: u64::MAX,
                    ..Limits::default()
                }
            )
            .unwrap_err()
            .code,
        "limits"
    );
    assert!(
        runtime
            .install("a", Some(2), &format!("{}{}", source(0), " ".repeat(512)))
            .is_err()
    );
}

mod native_fixture {
    include!("../examples/fixtures/control_flow_aot.rs");
    pub fn bind(
        runtime: &fluxr::runtime::Runtime,
        revision: u64,
    ) -> Result<(), fluxr::runtime::RuntimeError> {
        runtime.bind_aot(
            "compiled",
            revision,
            "walk",
            [
                209, 246, 134, 189, 246, 81, 14, 29, 9, 81, 152, 80, 120, 158, 0, 93, 53, 90, 73,
                143, 74, 12, 89, 71, 212, 34, 91, 52, 81, 213, 61, 4,
            ],
            kernel,
        )
    }
}
#[test]
fn owned_aot_is_a_revision_bound_runtime_tier_and_replacement_drops_stale_code() {
    let runtime = Runtime::new();
    let compact = include_str!("../examples/fixtures/control_flow.fluxr");
    let canonical = runtime::import_source(ImportKind::Compact, compact.as_bytes()).unwrap();
    runtime.install("compiled", None, &canonical).unwrap();
    let snapshot = runtime.snapshot("compiled").unwrap();
    // The existing checked artifact identity and released emitter are preserved.
    let module = Module::compile_compact(compact).unwrap();
    assert_eq!(snapshot.module().encode(), module.encode());
    assert_eq!(
        module.emit_native_rust("walk").unwrap(),
        include_str!("../examples/fixtures/control_flow_aot.rs")
    );
    assert!(
        module
            .emit_native_binding("walk")
            .unwrap()
            .contains("pub fn bind(")
    );
    native_fixture::bind(&runtime, 1).unwrap();
    let pinned = snapshot.prepare_export("walk").unwrap();
    let args = [
        Value::Record([Value::I64(7), Value::I64(-11)].into()),
        Value::I64(4),
        Value::I64(2),
    ];
    let result = snapshot
        .call("walk", &[], &args, Limits::default())
        .unwrap();
    assert_eq!(result.route, "aot");
    assert_eq!(
        result.value,
        Value::Record([Value::I64(6), Value::I64(-11)].into())
    );
    let mut workspace = Workspace::default();
    let pinned_result = pinned
        .call_with(&[], &args, Limits::default(), &mut workspace)
        .unwrap();
    assert_eq!(pinned_result.route, runtime::ExecutionRoute::AheadOfTime);
    assert_eq!(pinned_result.value, result.value);
    assert!(
        snapshot
            .call(
                "walk",
                &[],
                &args,
                Limits {
                    instructions: 0,
                    ..Limits::default()
                }
            )
            .is_err()
    );
    // Even a source-identical publication is a new revision with no implicit
    // binding reuse. A native descriptor must explicitly bind that revision.
    runtime.install("compiled", Some(1), &canonical).unwrap();
    assert!(runtime.inspect("compiled").unwrap().aot_exports.is_empty());
    assert_ne!(
        runtime
            .call("compiled", "walk", &[], &args, Limits::default())
            .unwrap()
            .route,
        "aot"
    );
    assert!(native_fixture::bind(&runtime, 1).is_err());
    assert_eq!(
        pinned
            .call_with(&[], &args, Limits::default(), &mut workspace)
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        snapshot
            .call("walk", &[], &args, Limits::default())
            .unwrap()
            .route,
        "aot"
    );
    native_fixture::bind(&runtime, 2).unwrap();
    // A body change with an unchanged interface rejects the old kernel digest.
    let mut changed = Document::parse(&canonical).unwrap().source;
    changed.functions[0].body = vec![fluxr::structured::Stmt::Return {
        value: fluxr::structured::Expr::I64 { value: 1 },
    }];
    let changed = Module::compile(changed).unwrap().unified_text().unwrap();
    runtime.install("compiled", Some(2), &changed).unwrap();
    assert!(native_fixture::bind(&runtime, 3).is_err());
    assert!(runtime.inspect("compiled").unwrap().aot_exports.is_empty());
    let prepared = native_fixture::prepare(&module).unwrap();
    assert_eq!(
        prepared
            .run_with(
                Invocation {
                    export: "walk",
                    inputs: &[],
                    arguments: &args,
                    limits: Limits::default()
                },
                &mut Workspace::default()
            )
            .unwrap(),
        result.value
    );
}

fn request(address: SocketAddr, payload: &[u8]) -> ControlResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(payload).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[test]
fn loopback_control_uses_the_same_strict_versioned_operation_handler() {
    let runtime = Arc::new(Runtime::new());
    let token = "0123456789abcdef0123456789abcdef";
    let service = ControlService::start(
        Arc::clone(&runtime),
        "127.0.0.1:0".parse().unwrap(),
        token.into(),
        None,
    )
    .unwrap();
    let make = |token: &str, version, command| {
        let mut bytes = serde_json::to_vec(&ControlRequest {
            version,
            token: token.into(),
            command,
        })
        .unwrap();
        bytes.push(b'\n');
        bytes
    };
    let install = || Command::Install {
        name: "remote".into(),
        expected_revision: None,
        source: source(42),
    };
    let rejected = request(service.address(), &make("incorrect", 1, install()));
    assert_eq!(rejected.error.unwrap().code, "authentication");
    assert!(runtime.list().unwrap().is_empty());
    assert_eq!(
        request(service.address(), &make(token, 9, install()))
            .error
            .unwrap()
            .code,
        "version"
    );
    assert_eq!(request(service.address(), &format!("{{\"version\":1,\"token\":\"{token}\",\"command\":{{\"op\":\"list\",\"unknown\":true}}}}\n").into_bytes()).error.unwrap().code, "invalid_request");
    assert!(request(service.address(), &make(token, 1, install())).ok);
    assert_eq!(call(&runtime, "remote").value, Value::U64(42));
    let remote = request(
        service.address(),
        &make(
            token,
            1,
            Command::Call {
                name: "remote".into(),
                export: "value".into(),
                inputs: vec![],
                arguments: vec![],
                limits: Limits::default(),
            },
        ),
    );
    assert!(remote.ok);
    assert_eq!(remote.result.unwrap()["revision"], 1);
    let address = service.address();
    drop(service);
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
    assert!(
        ControlService::start(
            Arc::clone(&runtime),
            "0.0.0.0:0".parse().unwrap(),
            token.into(),
            None
        )
        .is_err()
    );
    assert!(
        ControlService::start(
            runtime,
            "127.0.0.1:0".parse().unwrap(),
            "short".into(),
            None
        )
        .is_err()
    );
}
