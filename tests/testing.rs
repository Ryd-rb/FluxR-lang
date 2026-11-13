//! Executable regression tests for checked-source test selection and failure
//! accounting. These fixtures are independent of the cookbook's examples.
use fluxr::structured::Value;
use fluxr::testing::{self, Mode, Options, Status};
use serde_json::{Value as Json, json};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(source: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "fluxr-test-harness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("program.fluxr"), source).unwrap();
        Self { root }
    }
    fn catalog(&self, cases: Vec<Json>) -> PathBuf {
        let path = self.root.join("tests.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({"format":"fluxr-tests-v1","tests":cases})).unwrap(),
        )
        .unwrap();
        path
    }
    fn load(&self, cases: Vec<Json>) -> testing::Suite {
        testing::load(&self.catalog(cases)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn test(name: &str) -> Json {
    json!({"name":name,"source":"program.fluxr","export":"run","arguments":[{"i64":41}],"expect":{"value":{"i64":42}}})
}
const ADD: &str = "fluxr 1\npub run 1 x int int ret add x 1\n";

#[test]
fn exact_values_and_portable_prepared_routes_match() {
    let f = Fixture::new(ADD);
    let suite = f.load(vec![test("add")]);
    for mode in [Mode::Portable, Mode::Prepared, Mode::Compare] {
        let report = testing::run(
            &suite,
            &Options {
                mode,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(report.success);
        assert_eq!(report.passed, 1);
        assert_eq!(report.tests[0].executions, 1);
        assert!(!report.tests[0].routes.is_empty());
        assert_eq!(report.sources.len(), 1);
        assert_eq!(report.sources[0].bytes, ADD.len());
        assert_eq!(report.sources[0].sha256.len(), 64);
    }
}

#[test]
fn arithmetic_updates_execute_through_all_catalog_modes() {
    let f = Fixture::new(
        "fluxr 1\n\
         pub difference 2 x int y int int do 0 2 sub x y ret x\n\
         pub quotient 2 x int y int int do 0 2 div x y ret x\n\
         pub remainder 2 x int y int int do 0 2 rem x y ret x\n",
    );
    let mut cases = Vec::new();
    for (export, x, y, value) in [
        ("difference", 17i64, 5i64, 12i64),
        ("quotient", -17, 5, -3),
        ("remainder", -17, 5, -2),
    ] {
        cases.push(
            json!({"name":format!("{export} value"),"source":"program.fluxr",
            "export":export,"arguments":[{"i64":x},{"i64":y}],
            "expect":{"value":{"i64":value}}}),
        );
    }
    for (export, x, y) in [
        ("difference", i64::MIN, 1i64),
        ("quotient", 17, 0),
        ("remainder", 17, 0),
    ] {
        cases.push(
            json!({"name":format!("{export} trap"),"source":"program.fluxr",
            "export":export,"arguments":[{"i64":x},{"i64":y}],
            "expect":{"error":"invalid or overflowing integer arithmetic"}}),
        );
    }
    let suite = f.load(cases);
    for mode in [Mode::Portable, Mode::Prepared, Mode::Compare] {
        let report = testing::run(
            &suite,
            &Options {
                mode,
                repeat: 2,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(report.success, "{report:?}");
        assert_eq!((report.passed, report.failed, report.not_run), (6, 0, 0));
        for result in &report.tests {
            assert!(matches!(result.status, Status::Passed));
            assert_eq!(result.executions, 2);
            assert!(!result.routes.is_empty());
            if mode != Mode::Prepared {
                assert!(result.routes.iter().any(|route| route == "Portable"));
            }
        }
    }
}

#[test]
fn catalog_identity_binds_expectations_seeds_and_execution_options() {
    let f = Fixture::new(ADD);
    let source = f.load(vec![test("same")]);
    let first = testing::run(&source, &Options::default()).unwrap();
    let mut changed = test("same");
    changed["expect"] = json!({"value":{"i64":99}});
    let second = testing::run(&f.load(vec![changed]), &Options::default()).unwrap();
    assert_eq!(first.sources[0].sha256, second.sources[0].sha256);
    assert_ne!(first.catalog.sha256, second.catalog.sha256);
    assert!(first.catalog.bytes > 0);
    let options = Options {
        mode: Mode::Compare,
        repeat: 3,
        test_threads: 2,
        ..Options::default()
    };
    let repeated = testing::run(&source, &options).unwrap();
    assert_eq!(first.catalog.sha256, repeated.catalog.sha256);
    assert_eq!(repeated.configuration.repeat, 3);
    assert_eq!(repeated.configuration.test_threads, 2);
    assert_eq!(repeated.configuration.mode, Mode::Compare);
    let f = Fixture::new("fluxr 1\npub run 1 x int bool ret eq x x\n");
    let one = testing::run(
        &f.load(vec![property(
            "seed",
            1,
            4,
            json!([{"kind":"i64","min":0,"max":9}]),
        )]),
        &Options::default(),
    )
    .unwrap();
    let two = testing::run(
        &f.load(vec![property(
            "seed",
            2,
            4,
            json!([{"kind":"i64","min":0,"max":9}]),
        )]),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(one.sources[0].sha256, two.sources[0].sha256);
    assert_ne!(one.catalog.sha256, two.catalog.sha256);
}

#[test]
fn incorrect_values_and_missing_expected_errors_fail() {
    let f = Fixture::new(ADD);
    let mut wrong = test("wrong value");
    wrong["expect"] = json!({"value":{"i64":99}});
    let mut no_error = test("no error");
    no_error["expect"] = json!({"error":"overflow"});
    let report = testing::run(&f.load(vec![wrong, no_error]), &Options::default()).unwrap();
    assert!(!report.success);
    assert_eq!(report.failed, 2);
    assert_eq!(report.passed, 0);
}

#[test]
fn checked_overflow_and_zero_fuel_are_real_expected_errors() {
    let f = Fixture::new(ADD);
    let mut overflow = test("overflow");
    overflow["arguments"] = json!([{"i64":i64::MAX}]);
    overflow["expect"] = json!({"error":"overflow"});
    let mut fuel = test("fuel");
    fuel["limits"] = json!({"instructions":0,"cells":16384,"frames":64});
    fuel["expect"] = json!({"error":"instruction"});
    let report = testing::run(
        &f.load(vec![overflow, fuel]),
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.success, "{report:?}");
    assert_eq!(report.passed, 2);
}

#[test]
fn compile_errors_are_distinct_from_runtime_errors() {
    let f = Fixture::new("fluxr 1\npub run 0 int ret true\n");
    let mut compile = test("compile");
    compile["expect"] = json!({"compile_error":"expected I64, got Bool"});
    let mut runtime = test("runtime");
    runtime["expect"] = json!({"error":"expected I64, got Bool"});
    let report = testing::run(
        &f.load(vec![compile, runtime]),
        &Options {
            repeat: 7,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report.passed, 1, "{report:?}");
    assert_eq!(report.failed, 1);
    assert!(matches!(report.tests[0].status, Status::Passed));
    assert!(matches!(report.tests[1].status, Status::Failed));
    // Repeating runtime cases never invents executions for cached compilation
    // expectations, including the wrong-phase case that never reaches the VM.
    assert_eq!(report.configuration.repeat, 7);
    assert_eq!(report.tests[0].executions, 0);
    assert_eq!(report.tests[1].executions, 0);
}

#[test]
fn successful_compilation_cannot_satisfy_compile_error() {
    let f = Fixture::new(ADD);
    let mut case = test("compile");
    case["expect"] = json!({"compile_error":"type"});
    let report = testing::run(&f.load(vec![case]), &Options::default()).unwrap();
    assert!(!report.success);
    assert_eq!(report.failed, 1);
}

#[test]
fn unknown_export_cannot_satisfy_runtime_error() {
    let f = Fixture::new(ADD);
    let mut case = test("bad export");
    case["export"] = json!("missing");
    case["expect"] = json!({"error":"export"});
    let report = testing::run(&f.load(vec![case]), &Options::default()).unwrap();
    assert!(!report.success);
    assert!(
        report.tests[0]
            .detail
            .as_ref()
            .unwrap()
            .starts_with("harness:")
    );
}

#[test]
fn native_spans_cannot_be_silently_treated_as_checked_tests() {
    let native = "fn harmless() {}";
    let f = Fixture::new(&format!("{ADD}rust {}\n{native}", native.len()));
    let mut case = test("native");
    case["expect"] = json!({"compile_error":"native"});
    let report = testing::run(&f.load(vec![case]), &Options::default()).unwrap();
    assert!(!report.success);
    assert!(
        report.tests[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("cannot execute native")
    );
}

#[test]
fn listing_filters_and_ignore_modes_do_not_count_skips_as_passes() {
    let f = Fixture::new(ADD);
    let mut ignored = test("alpha ignored");
    ignored["ignore"] = json!("requires explicit selection");
    let suite = f.load(vec![test("alpha"), test("beta"), ignored]);
    let listed = testing::run(
        &suite,
        &Options {
            list: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(listed.listed);
    assert_eq!(listed.passed, 0);
    assert!(listed.tests.iter().all(|r| r.executions == 0));
    let report = testing::run(&suite, &Options::default()).unwrap();
    assert_eq!((report.passed, report.ignored, report.filtered), (2, 1, 0));
    let report = testing::run(
        &suite,
        &Options {
            filter: Some("alpha".into()),
            exact: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.passed, report.filtered), (1, 2));
    let report = testing::run(
        &suite,
        &Options {
            skip: vec!["alpha".into()],
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.passed, report.filtered), (1, 2));
    let report = testing::run(
        &suite,
        &Options {
            ignored_only: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.passed, report.ignored, report.filtered), (1, 0, 2));
    assert_eq!(
        testing::run(
            &suite,
            &Options {
                include_ignored: true,
                ..Options::default()
            }
        )
        .unwrap()
        .passed,
        3
    );
}

#[test]
fn listing_does_not_compile_invalid_source() {
    let f = Fixture::new("not a FluxR source");
    let report = testing::run(
        &f.load(vec![test("invalid")]),
        &Options {
            list: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.success);
    assert!(matches!(report.tests[0].status, Status::Listed));
}

#[test]
fn no_selection_is_an_error_unless_explicitly_allowed() {
    let f = Fixture::new(ADD);
    let suite = f.load(vec![test("add")]);
    let options = Options {
        filter: Some("missing".into()),
        ..Options::default()
    };
    assert!(testing::run(&suite, &options).is_err());
    assert!(
        testing::run(
            &suite,
            &Options {
                allow_empty: true,
                ..options
            }
        )
        .unwrap()
        .success
    );
}

#[test]
fn sequential_fail_fast_leaves_later_cases_unexecuted() {
    let f = Fixture::new(ADD);
    let mut first = test("first");
    first["expect"] = json!({"value":{"i64":0}});
    let report = testing::run(
        &f.load(vec![first, test("second"), test("third")]),
        &Options {
            fail_fast: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.failed, report.passed, report.not_run), (1, 0, 2));
    assert_eq!(report.tests[1].executions, 0);
}

#[test]
fn parallel_output_keeps_catalog_order_and_repeat_is_real() {
    let f = Fixture::new(ADD);
    let suite = f.load((0..32).map(|n| test(&format!("case {n:02}"))).collect());
    let report = testing::run(
        &suite,
        &Options {
            test_threads: 4,
            repeat: 3,
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.success);
    assert_eq!(report.passed, 32);
    for (n, result) in report.tests.iter().enumerate() {
        assert_eq!(result.name, format!("case {n:02}"));
        assert_eq!(result.executions, 3);
    }
}

fn property(name: &str, seed: u64, cases: usize, generators: Json) -> Json {
    json!({"name":name,"source":"program.fluxr","export":"run","expect":{"value":{"bool":true}},
        "property":{"seed":seed,"cases":cases,"generators":generators,"shrink":true}})
}

fn list_items(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        other => panic!("expected generated list, got {other:?}"),
    }
}

#[test]
fn scalar_seed_sequences_keep_the_released_case_order() {
    // Literal vectors from the incoming scalar generator, not an implementation
    // of SplitMix64 inside the test. Each predicate exposes a different case.
    let expected = [
        (
            true,
            -2_002_695_134_865_986_095i64,
            6_056_616_057_409_641_356u64,
        ),
        (true, -8_904_999_215_313_575_842, 1_502_954_475_416_400_618),
        (false, 2_832_123_623_268_534_069, 467_288_396_927_712_570),
        (false, -4_516_726_496_844_123_488, 1_475_254_212_943_182_506),
    ];
    for (index, &(flag, signed, unsigned)) in expected.iter().enumerate() {
        let f = Fixture::new(&format!(
            "fluxr 1\npub run 3 a bool b int c uint bool ret not eq b {signed}\n"
        ));
        let mut case = property(
            "scalar sequence",
            17,
            4,
            json!([
                {"kind":"bool"},
                {"kind":"i64","min":i64::MIN,"max":i64::MAX},
                {"kind":"u64","min":0,"max":u64::MAX}
            ]),
        );
        case["property"]["shrink"] = json!(false);
        let report = testing::run(&f.load(vec![case]), &Options::default()).unwrap();
        let result = &report.tests[0];
        let replay = result.replay.as_ref().unwrap();
        assert_eq!(replay.case, index);
        assert_eq!(replay.repetition, 0);
        assert_eq!(result.executions, index + 1);
        assert_eq!(
            replay.arguments,
            vec![Value::Bool(flag), Value::I64(signed), Value::U64(unsigned)]
        );
        assert_eq!(replay.arguments, replay.shrunk_arguments);
        assert_eq!(replay.shrink_evaluations, 0);
    }
}

#[test]
fn flat_list_generators_cover_types_lengths_and_full_width_domains() {
    use std::collections::BTreeSet;
    let f = Fixture::new(
        "fluxr 1\npub run 3 flags list bool signed list int unsigned list uint bool ret false\n",
    );
    let mut lengths = [BTreeSet::new(), BTreeSet::new(), BTreeSet::new()];
    let mut bools = BTreeSet::new();
    let mut signed_signs = BTreeSet::new();
    let mut unsigned_halves = BTreeSet::new();
    for seed in 0..48 {
        let mut case = property(
            "list domains",
            seed,
            1,
            json!([
                {"kind":"bool_list","min_len":0,"max_len":4},
                {"kind":"i64_list","min_len":0,"max_len":4,"min":i64::MIN,"max":i64::MAX},
                {"kind":"u64_list","min_len":0,"max_len":4,"min":0,"max":u64::MAX}
            ]),
        );
        case["property"]["shrink"] = json!(false);
        let report = testing::run(
            &f.load(vec![case]),
            &Options {
                mode: Mode::Compare,
                ..Options::default()
            },
        )
        .unwrap();
        let replay = report.tests[0].replay.as_ref().unwrap();
        assert!(
            report.tests[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("got Bool(false)")
        );
        assert_eq!(report.tests[0].executions, 1);
        for (index, value) in replay.arguments.iter().enumerate() {
            let items = list_items(value);
            assert!(items.len() <= 4);
            lengths[index].insert(items.len());
            for item in items {
                match (index, item) {
                    (0, Value::Bool(value)) => {
                        bools.insert(*value);
                    }
                    (1, Value::I64(value)) => {
                        signed_signs.insert(*value < 0);
                    }
                    (2, Value::U64(value)) => {
                        unsigned_halves.insert(*value > i64::MAX as u64);
                    }
                    _ => panic!("wrong generated element type: {item:?}"),
                }
            }
        }
        if seed == 17 {
            assert_eq!(
                serde_json::to_value(&replay.arguments).unwrap(),
                json!([
                    {"list":[{"bool":true},{"bool":false},{"bool":true},{"bool":false}]},
                    {"list":[{"i64":-8_644_757_664_351_751_164i64},{"i64":2_832_123_623_268_534_069i64},{"i64":-8_756_083_639_927_063_238i64}]},
                    {"list":[{"u64":4_706_645_540_010_652_320u64},{"u64":1_475_254_212_943_182_506u64},{"u64":7_552_487_339_472_427_397u64}]}
                ])
            );
        }
    }
    for observed in lengths {
        assert_eq!(observed, (0..=4).collect());
    }
    for observed in [bools, signed_signs, unsigned_halves] {
        assert_eq!(observed, [false, true].into_iter().collect());
    }
}

#[test]
fn list_replay_is_stable_and_can_be_copied_into_fixed_regressions() {
    let f = Fixture::new(
        "fluxr 1\npub run 4 a list bool b list int c list uint d int bool ret false\n",
    );
    let cases: Vec<_> = (0..3)
        .map(|seed| {
            property(
                &format!("replay {seed}"),
                seed,
                8,
                json!([
                    {"kind":"bool_list","min_len":0,"max_len":8},
                    {"kind":"i64_list","min_len":0,"max_len":8,"min":-100,"max":100},
                    {"kind":"u64_list","min_len":0,"max_len":8,"min":0,"max":100},
                    {"kind":"i64","min":-100,"max":100}
                ]),
            )
        })
        .collect();
    let suite = f.load(cases);
    let mut reference = None;
    for mode in [Mode::Portable, Mode::Prepared, Mode::Compare] {
        for test_threads in [1, 4] {
            let options = Options {
                mode,
                test_threads,
                repeat: 3,
                ..Options::default()
            };
            let first = testing::run(&suite, &options).unwrap();
            let second = testing::run(&suite, &options).unwrap();
            assert_eq!(
                serde_json::to_value(&first).unwrap(),
                serde_json::to_value(&second).unwrap()
            );
            let replays = serde_json::to_value(
                first
                    .tests
                    .iter()
                    .map(|test| test.replay.as_ref().unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            if let Some(reference) = &reference {
                assert_eq!(&replays, reference);
            } else {
                reference = Some(replays);
            }
            for result in &first.tests {
                let replay = result.replay.as_ref().unwrap();
                assert_eq!(replay.case, 0);
                assert_eq!(replay.repetition, 0);
                assert_eq!(result.executions, 1 + replay.shrink_evaluations);
                assert_eq!(
                    serde_json::to_value(&replay.shrunk_arguments).unwrap(),
                    json!([{"list":[]},{"list":[]},{"list":[]},{"i64":0}])
                );
                for arguments in [&replay.arguments, &replay.shrunk_arguments] {
                    let fixed = json!({"name":"fixed regression","source":"program.fluxr","export":"run",
                        "arguments":arguments,"expect":{"value":{"bool":true}}});
                    let fixed_report = testing::run(&f.load(vec![fixed]), &options).unwrap();
                    assert!(matches!(fixed_report.tests[0].status, Status::Failed));
                    assert_eq!(fixed_report.tests[0].detail, result.detail);
                    assert!(fixed_report.tests[0].replay.is_none());
                }
            }
        }
    }
}

#[test]
fn list_shrinking_preserves_false_results_and_exact_runtime_errors() {
    for (body, expected_detail) in [
        ("lt last xs 0", "got Bool(false)"),
        ("empty xs", "got Bool(false)"),
        (
            "eq div last xs 0 0",
            "invalid or overflowing integer arithmetic",
        ),
    ] {
        let f = Fixture::new(&format!("fluxr 1\npub run 1 xs list int bool ret {body}\n"));
        let suite = f.load(vec![property(
            "same failure",
            1,
            1,
            json!([{"kind":"i64_list","min_len":0,"max_len":8,"min":1,"max":9}]),
        )]);
        for mode in [Mode::Portable, Mode::Prepared, Mode::Compare] {
            let options = Options {
                mode,
                ..Options::default()
            };
            let report = testing::run(&suite, &options).unwrap();
            let result = &report.tests[0];
            assert!(
                result.detail.as_ref().unwrap().contains(expected_detail),
                "{report:?}"
            );
            let replay = result.replay.as_ref().unwrap();
            assert_eq!(list_items(&replay.arguments[0]).len(), 5);
            assert_eq!(list_items(&replay.shrunk_arguments[0]), &[Value::I64(1)]);
            assert!(replay.shrink_evaluations > 0 && replay.shrink_evaluations <= 128);
            assert_eq!(result.executions, 1 + replay.shrink_evaluations);
            let fixed = json!({"name":"retained failure","source":"program.fluxr","export":"run",
                "arguments":replay.shrunk_arguments,"expect":{"value":{"bool":true}}});
            let retained = testing::run(&f.load(vec![fixed]), &options).unwrap();
            assert_eq!(retained.tests[0].detail, result.detail);
        }
    }
}

#[test]
fn list_shrinking_finds_an_interior_witness_after_rejected_chunk_offsets() {
    let f = Fixture::new(
        "fluxr 1\npub run 1 xs list int bool do 0 2\n\
         while not empty xs 2 if eq last xs 0 1 ret false 0 pop xs\n\
         ret true\n",
    );
    let suite = f.load(vec![property(
        "interior witness",
        1,
        1,
        json!([{"kind":"i64_list","min_len":0,"max_len":8,"min":0,"max":9}]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    let replay = report.tests[0].replay.as_ref().unwrap();
    assert_eq!(
        list_items(&replay.arguments[0]),
        &[
            Value::I64(9),
            Value::I64(0),
            Value::I64(5),
            Value::I64(1),
            Value::I64(8)
        ]
    );
    assert_eq!(list_items(&replay.shrunk_arguments[0]), &[Value::I64(0)]);
    // Deleting everything or either of the first two size-two chunks loses
    // the witness. The third offset preserves it; subsequent reductions keep
    // the same failing value and the documented deterministic attempt order.
    assert_eq!(replay.shrink_evaluations, 10);
    assert_eq!(report.tests[0].executions, 11);
    assert!(
        report.tests[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("got Bool(false)")
    );
}

#[test]
fn list_integer_shrinking_tries_midpoints_after_a_passing_target() {
    for (ty, threshold, generator, original, reduced) in [
        (
            "int",
            "10",
            json!({"kind":"i64_list","min_len":1,"max_len":1,"min":0,"max":100}),
            Value::I64(35),
            Value::I64(18),
        ),
        (
            "uint",
            "10u",
            json!({"kind":"u64_list","min_len":1,"max_len":1,"min":0,"max":100}),
            Value::U64(35),
            Value::U64(17),
        ),
    ] {
        let f = Fixture::new(&format!(
            "fluxr 1\npub run 1 xs list {ty} bool ret lt last xs {threshold}\n"
        ));
        let suite = f.load(vec![property(
            "midpoint reduction",
            1,
            1,
            json!([generator]),
        )]);
        let report = testing::run(
            &suite,
            &Options {
                mode: Mode::Compare,
                ..Options::default()
            },
        )
        .unwrap();
        let replay = report.tests[0].replay.as_ref().unwrap();
        assert_eq!(list_items(&replay.arguments[0]), &[original]);
        assert_eq!(list_items(&replay.shrunk_arguments[0]), &[reduced]);
        // Zero passes, so only the failing midpoint is retained. Its next
        // midpoint passes too: this bounded reducer does not claim minimality.
        assert_eq!(replay.shrink_evaluations, 4);
        assert_eq!(report.tests[0].executions, 5);
        assert!(
            report.tests[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("got Bool(false)")
        );
    }
}

#[test]
fn list_shrinking_stays_within_nonzero_length_and_element_bounds() {
    let f = Fixture::new("fluxr 1\npub run 3 a list bool b list int c list uint bool ret false\n");
    let suite = f.load(vec![property(
        "bounded reduction",
        1,
        1,
        json!([
            {"kind":"bool_list","min_len":2,"max_len":8},
            {"kind":"i64_list","min_len":2,"max_len":8,"min":-100,"max":-10},
            {"kind":"u64_list","min_len":2,"max_len":8,"min":7,"max":90}
        ]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    let replay = report.tests[0].replay.as_ref().unwrap();
    assert_eq!(
        serde_json::to_value(&replay.shrunk_arguments).unwrap(),
        json!([
            {"list":[{"bool":false},{"bool":false}]},
            {"list":[{"i64":-10},{"i64":-10}]},
            {"list":[{"u64":7},{"u64":7}]}
        ])
    );
    assert!(replay.shrink_evaluations <= 128);
}

#[test]
fn list_shrink_evaluation_bound_is_shared_across_arguments() {
    let f = Fixture::new("fluxr 1\npub run 3 a list bool b list bool c uint bool ret false\n");
    let suite = f.load(vec![property(
        "global shrink bound",
        17,
        1,
        json!([
            {"kind":"bool_list","min_len":128,"max_len":128},
            {"kind":"bool_list","min_len":512,"max_len":512},
            {"kind":"u64","min":0,"max":u64::MAX}
        ]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    let replay = report.tests[0].replay.as_ref().unwrap();
    assert_eq!(replay.shrink_evaluations, 128);
    assert_eq!(report.tests[0].executions, 129);
    let mut changed = 0;
    for index in 0..2 {
        let original = list_items(&replay.arguments[index]);
        let shrunk = list_items(&replay.shrunk_arguments[index]);
        assert_eq!(original.len(), shrunk.len());
        for (before, after) in original.iter().zip(shrunk) {
            if before != after {
                assert_eq!((before, after), (&Value::Bool(true), &Value::Bool(false)));
                changed += 1;
            }
        }
    }
    assert_eq!(changed, 128);
    assert!(
        list_items(&replay.shrunk_arguments[0])
            .iter()
            .all(|value| *value == Value::Bool(false))
    );
    assert_eq!(replay.arguments[2], replay.shrunk_arguments[2]);
}

#[test]
fn fixed_empty_lists_are_typed_and_need_no_shrink_evaluations() {
    let f = Fixture::new("fluxr 1\npub run 3 a list bool b list int c list uint bool ret false\n");
    let suite = f.load(vec![property(
        "empty lists",
        u64::MAX,
        1,
        json!([
            {"kind":"bool_list","min_len":0,"max_len":0},
            {"kind":"i64_list","min_len":0,"max_len":0,"min":i64::MIN,"max":i64::MIN},
            {"kind":"u64_list","min_len":0,"max_len":0,"min":u64::MAX,"max":u64::MAX}
        ]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    let result = &report.tests[0];
    assert!(result.detail.as_ref().unwrap().contains("got Bool(false)"));
    let replay = result.replay.as_ref().unwrap();
    assert_eq!(replay.shrink_evaluations, 0);
    assert_eq!(result.executions, 1);
    assert_eq!(replay.arguments, replay.shrunk_arguments);
    assert!(
        replay
            .arguments
            .iter()
            .all(|value| list_items(value).is_empty())
    );
}

#[test]
fn list_length_and_aggregate_cell_caps_are_admitted_before_generation() {
    let f = Fixture::new("fluxr 1\npub run 1 xs list bool bool ret not empty xs\n");
    let suite = f.load(vec![property(
        "maximum length",
        1,
        1,
        json!([{"kind":"bool_list","min_len":4096,"max_len":4096}]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.success, "{report:?}");
    assert_eq!(report.tests[0].executions, 1);

    // A list wrapper is a cell too: 64 * (1 + 4095) reaches the exact cap.
    // Loading validates this independently of the later export signature.
    let mut generators = vec![json!({"kind":"bool_list","min_len":0,"max_len":4095}); 64];
    assert!(
        testing::load(&f.catalog(vec![property("exact cells", 0, 1, json!(generators))])).is_ok()
    );
    generators[0]["max_len"] = json!(4096);
    assert!(
        testing::load(&f.catalog(vec![property("one excess cell", 0, 1, json!(generators))]))
            .is_err()
    );
    for generator in [
        json!({"kind":"bool_list","min_len":0,"max_len":4097}),
        json!({"kind":"bool_list","min_len":5,"max_len":4}),
        json!({"kind":"bool_list","min_len":0,"max_len":u64::MAX}),
        json!({"kind":"bool_list","min_len":-1,"max_len":1}),
        json!({"kind":"i64_list","min_len":0,"max_len":0,"min":5,"max":1}),
        json!({"kind":"u64_list","min_len":0,"max_len":0,"min":5,"max":1}),
        json!({"kind":"i64_list","min_len":0,"max_len":1,"max":1}),
        json!({"kind":"bool_list","min_len":0,"max_len":1,"element":{"kind":"bool"}}),
        json!({"kind":"list","min_len":0,"max_len":1,"element":{"kind":"bool_list","min_len":0,"max_len":1}}),
    ] {
        assert!(
            testing::load(&f.catalog(vec![property(
                "invalid list",
                0,
                1,
                json!([generator.clone()])
            )]))
            .is_err(),
            "accepted {generator}"
        );
    }
    fs::write(f.root.join("tests.json"), r#"{"format":"fluxr-tests-v1","tests":[{"name":"duplicate length","source":"program.fluxr","export":"run","expect":{"value":{"bool":true}},"property":{"seed":0,"cases":1,"generators":[{"kind":"bool_list","min_len":0,"min_len":1,"max_len":1}]}}]}"#).unwrap();
    assert!(
        testing::load(&f.root.join("tests.json"))
            .err()
            .unwrap()
            .to_string()
            .contains("duplicate JSON key")
    );
}

#[test]
fn generated_lists_keep_runtime_cell_limits_distinct_from_generation_caps() {
    for (ty, generator) in [
        ("bool", json!({"kind":"bool_list","min_len":4,"max_len":4})),
        (
            "int",
            json!({"kind":"i64_list","min_len":4,"max_len":4,"min":1,"max":9}),
        ),
        (
            "uint",
            json!({"kind":"u64_list","min_len":4,"max_len":4,"min":1,"max":9}),
        ),
    ] {
        let f = Fixture::new(&format!("fluxr 1\npub run 1 xs list {ty} bool ret true\n"));
        for cells in [0, 7, 8] {
            let mut case = property("runtime cells", 1, 1, json!([generator.clone()]));
            case["property"]["shrink"] = json!(false);
            case["limits"] = json!({"instructions":100,"cells":cells,"frames":64});
            let suite = f.load(vec![case]);
            for mode in [Mode::Portable, Mode::Prepared, Mode::Compare] {
                let report = testing::run(
                    &suite,
                    &Options {
                        mode,
                        ..Options::default()
                    },
                )
                .unwrap();
                assert_eq!(report.tests[0].executions, 1);
                // Five input cells, two frame cells, one returned Boolean.
                assert_eq!(report.success, cells == 8, "{ty}, {cells}: {report:?}");
                if cells < 8 {
                    assert!(
                        report.tests[0]
                            .detail
                            .as_ref()
                            .unwrap()
                            .contains("module cell budget exhausted")
                    );
                }
            }
        }
    }
}

#[test]
fn list_properties_reserve_exact_invocation_repeat_compare_and_shrink_limits() {
    let f = Fixture::new("fluxr 1\npub run 1 xs list bool bool ret true\n");
    for shrink in [false, true] {
        for mode in [Mode::Prepared, Mode::Compare] {
            let cases = if shrink { 896 } else { 1024 };
            let repeat = if mode == Mode::Compare { 32 } else { 64 };
            let mut case = property(
                "invocation boundary",
                1,
                cases,
                json!([{"kind":"bool_list","min_len":0,"max_len":0}]),
            );
            case["property"]["shrink"] = json!(shrink);
            // Stop at the first actual call while still exercising admission
            // of the entire 65,536-call reservation; no fuel cap confounds it.
            case["limits"] = json!({"instructions":0,"cells":100,"frames":64});
            let options = Options {
                mode,
                repeat,
                ..Options::default()
            };
            let suite = f.load(vec![case.clone()]);
            let report = testing::run(&suite, &options).unwrap();
            assert_eq!(report.tests[0].executions, 1);
            assert!(
                report.tests[0]
                    .detail
                    .as_ref()
                    .unwrap()
                    .contains("instruction budget exhausted")
            );
            assert!(
                testing::run(
                    &suite,
                    &Options {
                        repeat: repeat + 1,
                        ..options.clone()
                    }
                )
                .is_err()
            );
            if shrink {
                case["property"]["cases"] = json!(cases + 1);
                assert!(testing::run(&f.load(vec![case]), &options).is_err());
            }
        }
    }
}

#[test]
fn generated_cell_work_reserves_length_cases_shrink_repeats_and_comparison() {
    let f = Fixture::new("fluxr 1\npub run 1 xs list bool bool ret true\n");
    for (mode, cases, shrink, repeat) in [
        (Mode::Prepared, 1024, false, 1),
        (Mode::Compare, 512, false, 1),
        (Mode::Compare, 384, true, 1),
        (Mode::Prepared, 256, false, 4),
    ] {
        // Reserve 4,096 maximum argument cells per call, even though seed 6
        // generates an empty first list. Zero fuel stops after that one call.
        let mut large = property(
            "exact cell work",
            6,
            cases,
            json!([{"kind":"bool_list","min_len":0,"max_len":4095}]),
        );
        large["property"]["shrink"] = json!(shrink);
        large["limits"] = json!({"instructions":0,"cells":16384,"frames":64});
        let options = Options {
            mode,
            repeat,
            ..Options::default()
        };
        let report = testing::run(&f.load(vec![large.clone()]), &options).unwrap();
        assert_eq!(report.tests[0].executions, 1);
        assert!(list_items(&report.tests[0].replay.as_ref().unwrap().arguments[0]).is_empty());
        assert!(
            report.tests[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("instruction budget exhausted")
        );

        let mut extra = property(
            "extra cell work",
            6,
            1,
            json!([{"kind":"bool_list","min_len":0,"max_len":0}]),
        );
        extra["property"]["shrink"] = json!(false);
        extra["limits"] = large["limits"].clone();
        // One more empty-list wrapper exceeds the exact 4,194,304-cell
        // reservation while invocation and instruction limits remain below cap.
        assert!(
            testing::run(&f.load(vec![large.clone(), extra]), &options)
                .unwrap_err()
                .to_string()
                .contains("4,194,304 generated argument cell-work reservation")
        );

        let fixed = json!({"name":"fixed empty list","source":"program.fluxr","export":"run",
            "arguments":[{"list":[]}],"expect":{"value":{"bool":true}},"limits":large["limits"]});
        // Fixed test values retain their existing admission contract; they do
        // not consume the new generated-argument reservation.
        let report = testing::run(&f.load(vec![large, fixed]), &options).unwrap();
        assert_eq!(report.failed, 2);
    }
}

#[test]
fn generated_cell_work_honors_selection_and_preserves_scalar_admission() {
    let f = Fixture::new("fluxr 1\npub run 1 xs list bool bool ret true\n");
    let mut large = property(
        "large",
        6,
        1024,
        json!([{"kind":"bool_list","min_len":0,"max_len":4095}]),
    );
    large["property"]["shrink"] = json!(false);
    large["limits"] = json!({"instructions":0,"cells":16384,"frames":64});
    let mut extra = large.clone();
    extra["name"] = json!("extra");
    extra["property"]["cases"] = json!(1);
    extra["property"]["generators"][0]["max_len"] = json!(0);
    let suite = f.load(vec![large.clone(), extra.clone()]);
    for options in [
        Options {
            skip: vec!["extra".into()],
            ..Options::default()
        },
        Options {
            filter: Some("large".into()),
            exact: true,
            ..Options::default()
        },
    ] {
        let report = testing::run(&suite, &options).unwrap();
        assert_eq!((report.selected, report.filtered, report.failed), (1, 1, 1));
    }
    let listed = testing::run(
        &suite,
        &Options {
            list: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(listed.listed && listed.success);
    assert!(listed.tests.iter().all(|test| test.executions == 0));
    extra["ignore"] = json!("explicit selection required");
    let suite = f.load(vec![large, extra.clone()]);
    let report = testing::run(&suite, &Options::default()).unwrap();
    assert_eq!((report.failed, report.ignored), (1, 1));
    assert!(
        testing::run(
            &suite,
            &Options {
                include_ignored: true,
                ..Options::default()
            }
        )
        .is_err()
    );
    let report = testing::run(
        &suite,
        &Options {
            ignored_only: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((report.selected, report.failed), (1, 1));
    extra["property"]["generators"][0]["max_len"] = json!(4097);
    assert!(testing::load(&f.catalog(vec![extra])).is_err());

    // The largest formerly admitted scalar-only run reaches the new cap
    // exactly: 64 scalar arguments * 65,536 reserved backend invocations.
    let params = (0..64)
        .map(|n| format!("x{n} bool"))
        .collect::<Vec<_>>()
        .join(" ");
    let f = Fixture::new(&format!("fluxr 1\npub run 64 {params} bool ret true\n"));
    let mut scalar = property(
        "scalar admission",
        17,
        1024,
        json!(vec![json!({"kind":"bool"}); 64]),
    );
    scalar["property"]["shrink"] = json!(false);
    scalar["limits"] = json!({"instructions":0,"cells":16384,"frames":64});
    let report = testing::run(
        &f.load(vec![scalar]),
        &Options {
            repeat: 64,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(report.tests[0].executions, 1);
    assert_eq!(report.tests[0].replay.as_ref().unwrap().arguments.len(), 64);
    assert!(
        report.tests[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("instruction budget exhausted")
    );
}

#[test]
fn list_element_and_nesting_signature_mismatches_never_invoke() {
    for (ty, generator) in [
        ("bool", json!({"kind":"bool_list","min_len":0,"max_len":0})),
        (
            "list int",
            json!({"kind":"bool_list","min_len":0,"max_len":0}),
        ),
        (
            "list bool",
            json!({"kind":"i64_list","min_len":0,"max_len":0,"min":0,"max":0}),
        ),
        (
            "list list int",
            json!({"kind":"i64_list","min_len":0,"max_len":0,"min":0,"max":0}),
        ),
        ("list uint", json!({"kind":"u64","min":0,"max":0})),
    ] {
        let f = Fixture::new(&format!("fluxr 1\npub run 1 xs {ty} bool ret true\n"));
        let report = testing::run(
            &f.load(vec![property("wrong list type", 1, 1, json!([generator]))]),
            &Options::default(),
        )
        .unwrap();
        let result = &report.tests[0];
        assert!(matches!(result.status, Status::Failed));
        assert_eq!(result.executions, 0);
        assert!(result.replay.is_none());
        assert!(
            result
                .detail
                .as_ref()
                .unwrap()
                .starts_with("harness: property generators"),
            "{report:?}"
        );
    }
}

#[test]
fn cli_list_properties_report_reproducible_original_and_reduced_arguments() {
    let f = Fixture::new("fluxr 1\npub run 1 xs list int bool ret lt last xs 0\n");
    let path = f.catalog(vec![property(
        "CLI list replay",
        1,
        8,
        json!([{"kind":"i64_list","min_len":0,"max_len":8,"min":1,"max":9}]),
    )]);
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
            .args([
                "test-suite",
                path.to_str().unwrap(),
                "--format",
                "json",
                "--mode",
                "compare",
            ])
            .output()
            .unwrap()
    };
    let first = invoke();
    let second = invoke();
    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stdout, second.stdout);
    let report: Json = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["format"], "fluxr-test-results-v1");
    assert_eq!(report["failed"], 1);
    let replay = &report["tests"][0]["replay"];
    assert_eq!(replay["seed"], 1);
    assert_eq!(replay["case"], 0);
    assert_eq!(replay["repetition"], 0);
    assert_eq!(replay["arguments"][0]["list"].as_array().unwrap().len(), 5);
    assert_eq!(replay["shrunk_arguments"], json!([{"list":[{"i64":1}]}]));
    assert!(replay["shrink_evaluations"].as_u64().unwrap() <= 128);
    assert_eq!(
        report["tests"][0]["executions"].as_u64().unwrap(),
        1 + replay["shrink_evaluations"].as_u64().unwrap()
    );
}

#[test]
fn seeded_properties_cover_full_width_integer_domains_without_overflow() {
    let f = Fixture::new(
        "fluxr 1\npub run 3 a int b uint c bool bool ret and eq a a and eq b b eq c c\n",
    );
    let suite = f.load(vec![property(
        "reflexive",
        u64::MAX,
        128,
        json!([
        {"kind":"i64","min":i64::MIN,"max":i64::MAX},
        {"kind":"u64","min":0,"max":u64::MAX},{"kind":"bool"}]),
    )]);
    let report = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.success, "{report:?}");
    assert_eq!(report.tests[0].executions, 128);
}

#[test]
fn failing_property_replays_and_shrinks_within_its_domain() {
    let f = Fixture::new("fluxr 1\npub run 1 x int bool ret lt x 10\n");
    let suite = f.load(vec![property(
        "small",
        17,
        32,
        json!([{"kind":"i64","min":10,"max":1000}]),
    )]);
    let first = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    let second = testing::run(
        &suite,
        &Options {
            mode: Mode::Compare,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!first.success);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    let replay = first.tests[0].replay.as_ref().unwrap();
    assert_eq!(replay.seed, 17);
    assert_eq!(replay.case, 0);
    assert_eq!(replay.shrunk_arguments, vec![Value::I64(10)]);
    assert!(replay.shrink_evaluations <= 128);
}

#[test]
fn malformed_property_signature_is_harness_failure() {
    let f = Fixture::new(ADD);
    let suite = f.load(vec![property(
        "wrong signature",
        1,
        10,
        json!([{"kind":"i64","min":0,"max":9}]),
    )]);
    let report = testing::run(&suite, &Options::default()).unwrap();
    assert!(!report.success);
    assert_eq!(report.tests[0].executions, 0);
}

#[test]
fn duplicate_keys_at_every_level_are_rejected() {
    let f = Fixture::new(ADD);
    for source in [
        r#"{"format":"fluxr-tests-v1","format":"fluxr-tests-v1","tests":[]}"#,
        r#"{"format":"fluxr-tests-v1","tests":[{"name":"x","name":"x","source":"program.fluxr","export":"run","expect":{"value":{"i64":1}}}]}"#,
        r#"{"format":"fluxr-tests-v1","tests":[{"name":"x","source":"program.fluxr","export":"run","expect":{"value":{"i64":1},"value":{"i64":2}}}]}"#,
    ] {
        fs::write(f.root.join("tests.json"), source).unwrap();
        assert!(
            testing::load(&f.root.join("tests.json"))
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate JSON key")
        );
    }
}

#[test]
fn unknown_fields_bad_expectations_and_unbounded_options_are_rejected() {
    let f = Fixture::new(ADD);
    let mut cases = vec![];
    let mut unknown = test("unknown");
    unknown["typo"] = json!(true);
    cases.push(unknown);
    let mut multiple = test("multiple");
    multiple["expect"] = json!({"value":{"i64":42},"error":"bad"});
    cases.push(multiple);
    let mut empty = test("empty");
    empty["expect"] = json!({"error":""});
    cases.push(empty);
    let mut empty_ignore = test("empty ignore");
    empty_ignore["ignore"] = json!("");
    cases.push(empty_ignore);
    let mut limit = test("limit");
    limit["limits"] = json!({"instructions":1000001,"cells":100,"frames":10});
    cases.push(limit);
    let mut ignored_bool = property("bool unknown", 1, 1, json!([{"kind":"bool","extra":true}]));
    ignored_bool["export"] = json!("run");
    cases.push(ignored_bool);
    let mut bad_range = property("range", 1, 1, json!([{"kind":"i64","min":5,"max":1}]));
    bad_range["export"] = json!("run");
    cases.push(bad_range);
    for case in cases {
        assert!(
            testing::load(&f.catalog(vec![case.clone()])).is_err(),
            "accepted {case}"
        );
    }
    assert!(testing::load(&f.catalog(vec![test("same"), test("same")])).is_err());
    let suite = f.load(vec![test("ok")]);
    for options in [
        Options {
            repeat: 0,
            ..Options::default()
        },
        Options {
            test_threads: 9,
            ..Options::default()
        },
        Options {
            ignored_only: true,
            include_ignored: true,
            ..Options::default()
        },
    ] {
        assert!(testing::run(&suite, &options).is_err());
    }
}

#[test]
fn aggregate_property_work_cannot_bypass_execution_reservation() {
    let f = Fixture::new("fluxr 1\npub run 1 x int bool ret eq x x\n");
    let suite = f.load(vec![property(
        "large",
        1,
        1024,
        json!([{"kind":"i64","min":0,"max":1}]),
    )]);
    assert!(
        testing::run(
            &suite,
            &Options {
                repeat: 100,
                ..Options::default()
            }
        )
        .is_err()
    );
}

#[test]
fn unsafe_and_missing_source_paths_fail_before_execution() {
    let f = Fixture::new(ADD);
    for path in [
        "../program.fluxr",
        "/program.fluxr",
        "./program.fluxr",
        "x//y",
        "x\\y",
        "C:program.fluxr",
        "missing.fluxr",
    ] {
        let mut case = test("path");
        case["source"] = json!(path);
        case["expect"] = json!({"compile_error":"file"});
        assert!(
            testing::load(&f.catalog(vec![case])).is_err(),
            "accepted {path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn source_symlinks_and_catalog_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(ADD);
    symlink(f.root.join("program.fluxr"), f.root.join("link.fluxr")).unwrap();
    let mut case = test("link");
    case["source"] = json!("link.fluxr");
    assert!(testing::load(&f.catalog(vec![case])).is_err());
    let catalog = f.catalog(vec![test("plain")]);
    symlink(catalog, f.root.join("link.json")).unwrap();
    assert!(testing::load(&f.root.join("link.json")).is_err());
}

#[test]
fn oversized_catalog_and_sources_are_rejected_before_parsing() {
    let f = Fixture::new(ADD);
    let path = f.catalog(vec![test("big")]);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(testing::load(&path).is_err());
    let path = f.catalog(vec![test("big")]);
    fs::OpenOptions::new()
        .write(true)
        .open(f.root.join("program.fluxr"))
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(testing::load(&path).is_err());
}

#[test]
fn cli_runs_real_tests_and_emits_machine_readable_failures() {
    let f = Fixture::new(ADD);
    let path = f.catalog(vec![test("ok")]);
    let output = Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
        .args([
            "test-suite",
            path.to_str().unwrap(),
            "--format",
            "json",
            "--mode",
            "compare",
            "--repeat",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["tests"][0]["executions"], 2);
    let mut failing = test("bad");
    failing["expect"] = json!({"value":{"i64":0}});
    f.catalog(vec![failing]);
    let output = Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
        .args(["test-suite", path.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["failed"], 1);
    assert_eq!(report["success"], false);
}

#[test]
fn cli_rejects_unknown_flags_and_empty_filters_and_lists_without_running() {
    let f = Fixture::new(ADD);
    let path = f.catalog(vec![test("ok")]);
    let binary = env!("CARGO_BIN_EXE_fluxr-cli");
    for tail in [
        vec!["missing"],
        vec!["--test-threads", "0"],
        vec!["--repeat", "101"],
        vec!["--unknown"],
        vec!["--format", "xml"],
    ] {
        assert!(
            !Command::new(binary)
                .arg("test-suite")
                .arg(&path)
                .args(tail)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let output = Command::new(binary)
        .arg("test-suite")
        .arg(&path)
        .args(["--list", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["listed"], true);
    assert_eq!(report["passed"], 0);
}

#[test]
fn checked_test_suites_coexist_with_released_rule_and_timeline_replay() {
    let f = Fixture::new(ADD);
    fs::write(f.root.join("rule.flux"), "r4 1 i0").unwrap();
    fs::write(f.root.join("rule.json"), "[[[7],7],[[0],0]]").unwrap();
    fs::write(f.root.join("timeline.flux"), "v1 10\n1 1 1 0:0 10:100").unwrap();
    // Released timelines sample time modulo duration: the interior is linear,
    // and the exact period endpoint wraps to the initial key.
    fs::write(
        f.root.join("timeline.json"),
        "[[1,1,0,0],[1,1,5,50],[1,1,10,0]]",
    )
    .unwrap();
    for (source, replay, expected) in [
        ("rule.flux", "rule.json", "2 rule assertions passed\n"),
        (
            "timeline.flux",
            "timeline.json",
            "3 scenario assertions passed\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
            .current_dir(&f.root)
            .args(["test", source, replay])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    let catalog = f.catalog(vec![test("checked")]);
    let output = Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
        .args(["test-suite", catalog.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let report: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["failed"], 0);
}
