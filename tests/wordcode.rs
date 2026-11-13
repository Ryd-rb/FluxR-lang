use fluxr::structured::wordcode::{
    self, Definition, Dictionary, ExpansionLimits, ScalarType, Template,
};
use fluxr::structured::{Binary, Invocation, Limits, Module, Value};

fn wk(text: &str) -> Module {
    Module::compile_wordcode(text, &Dictionary::core(), ExpansionLimits::default()).unwrap()
}

fn run(module: &Module, args: &[i64], limits: Limits) -> String {
    let values: Vec<_> = args.iter().copied().map(Value::I64).collect();
    format!(
        "{:?}",
        module.run(Invocation {
            export: "calc",
            inputs: &[],
            arguments: &values,
            limits
        })
    )
}

fn equivalent(wordcode: &str, expanded: &str, values: &[Vec<i64>]) {
    let word = wk(wordcode);
    let plain = Module::compile_agent(expanded).unwrap();
    assert_eq!(word.encode(), plain.encode());
    for args in values {
        for instructions in 0..80 {
            for cells in [0, 1, 3, 8, 64] {
                for frames in [0, 1, 4] {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    assert_eq!(
                        run(&word, args, limits),
                        run(&plain, args, limits),
                        "args={args:?} limits={limits:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn core_scalar_words_preserve_checked_artifacts_and_all_budget_outcomes() {
    let cases = [
        ("up x", "add x 1"),
        ("down x", "sub x 1"),
        ("twice x", "mul x 2"),
        ("square x", "mul x x"),
        ("cube x", "mul mul x x x"),
        ("four x", "mul mul x 2 2"),
        ("step x y z", "add mul x y z"),
        ("sum x y z", "add add x y z"),
        ("ratio x y z", "div add x y z"),
    ];
    let values = vec![
        vec![4, 2, 3],
        vec![i64::MAX, 1, 0],
        vec![i64::MIN, -1, -1],
        vec![0, 0, 0],
    ];
    for (compressed, expanded) in cases {
        equivalent(
            &format!("wk1 core v2 pub calc 3 x int y int z int int ret {compressed}"),
            &format!("fa1 v2 pub calc 3 x int y int z int int ret {expanded}"),
            &values,
        );
    }
}

#[test]
fn bool_and_float_words_preserve_exact_bits() {
    for (word, expanded) in [
        ("zero x", "eq x 0"),
        ("positive x", "lt 0 x"),
        ("negative x", "lt x 0"),
        ("between y x z", "and le y x lt x z"),
    ] {
        equivalent(
            &format!("wk1 core v2 pub calc 3 x int y int z int bool ret {word}"),
            &format!("fa1 v2 pub calc 3 x int y int z int bool ret {expanded}"),
            &[
                vec![1, 0, 2],
                vec![-1, 0, 1],
                vec![i64::MAX, i64::MIN, i64::MAX],
            ],
        );
    }
    for (word, expanded) in [("blend x y z", "add mul x y z"), ("root x", "sqrt abs x")] {
        let a = wk(&format!(
            "wk1 core v2 pub calc 3 x float y float z float float ret {word}"
        ));
        let b = Module::compile_agent(&format!(
            "fa1 v2 pub calc 3 x float y float z float float ret {expanded}"
        ))
        .unwrap();
        assert_eq!(a.encode(), b.encode());
        for bits in [
            0, 0x80000000, 1, 0x00800000, 0x3f800000, 0x7f800000, 0x7fc12345,
        ] {
            let args = [Value::F32(bits), Value::F32(0x3f800000), Value::F32(0)];
            for instructions in 0..24 {
                let call = |m: &Module| {
                    format!(
                        "{:?}",
                        m.run(Invocation {
                            export: "calc",
                            inputs: &[],
                            arguments: &args,
                            limits: Limits {
                                instructions,
                                cells: 20,
                                frames: 3
                            }
                        })
                    )
                };
                assert_eq!(call(&a), call(&b));
            }
        }
    }
}

#[test]
fn statement_words_preserve_order_overflow_and_nonprogress_fuel() {
    equivalent(
        "wk1 core v2 pub calc 3 x int n int s int int do 0 4 next x bump x 2 scale x 3 ret x",
        "fa1 v2 pub calc 3 x int n int s int int do 0 4 set x add x 1 set x add x 2 set x mul x 3 ret x",
        &[vec![0, 0, 0], vec![i64::MAX, 0, 0], vec![i64::MIN, 0, 0]],
    );
    equivalent(
        "wk1 core v2 pub calc 3 x int n int s int int do 0 2 walk x n s ret x",
        "fa1 v2 pub calc 3 x int n int s int int do 0 2 while lt x n 1 set x add x s ret x",
        &[
            vec![0, 4, 1],
            vec![0, 4, 0],
            vec![0, 4, -1],
            vec![i64::MAX - 1, i64::MAX, 2],
        ],
    );
}

#[test]
fn repeat_is_hygienic_evaluates_count_once_and_continue_progresses() {
    let text = "wk1 core v2 pub calc 1 n int int do 1 x int 3 set x 0 repeat n 3 next x set n 0 continue ret x";
    let expected = "fa1 v2 pub calc 1 n int int do 2 x int wk_private_0 int 4 set x 0 set wk_private_0 n while lt 0 wk_private_0 4 set wk_private_0 sub wk_private_0 1 set x add x 1 set n 0 continue ret x";
    equivalent(
        text,
        expected,
        &[vec![-1], vec![0], vec![1], vec![4], vec![i64::MAX]],
    );
    assert_eq!(run(&wk(text), &[5], Limits::default()), "Ok(I64(5))");
}

#[test]
fn repeat_nested_break_and_private_names_cannot_capture() {
    let text = "wk1 core v2 pub calc 0 int do 1 x int 3 set x 0 repeat 3 2 repeat 4 2 next x break next x ret x";
    let result = wordcode::compile(text, &Dictionary::core(), ExpansionLimits::default()).unwrap();
    assert_eq!(result.report.generated_locals, 2);
    assert_eq!(run(&result.module, &[], Limits::default()), "Ok(I64(6))");
    for text in [
        "wk1 core v2 pub calc 1 wk_private_0 int int ret 0",
        "wk1 core v2 pub calc 0 int ret wk_private_0",
        "wk1 core v2 pub calc 0 int ret local wk_private_0",
        "wk1 core v2 pub calc 0 int ret input wk_private_0",
    ] {
        assert!(
            Module::compile_wordcode(text, &Dictionary::core(), ExpansionLimits::default())
                .is_err()
        );
    }
}

#[test]
fn profile_and_dictionary_pins_are_required_and_released_fa1_is_unchanged() {
    for text in [
        "fa1 v2 pub calc 0 int ret up 1",
        "wk2 core v2 pub calc 0 int ret 1",
        "wk1 other v2 pub calc 0 int ret 1",
        "wk1 core v3 pub calc 0 int ret 1",
    ] {
        assert!(
            Module::compile_wordcode(text, &Dictionary::core(), ExpansionLimits::default())
                .is_err()
        );
    }
    assert!(Module::compile_agent("wk1 core v2 pub calc 0 int ret 1").is_err());
    assert_eq!(
        run(
            &Module::compile_agent("fa1 v2 pub calc 1 up int int ret up").unwrap(),
            &[7],
            Limits::default()
        ),
        "Ok(I64(7))"
    );
    assert_eq!(
        run(
            &wk("wk1 core v2 pub calc 1 up int int ret local up"),
            &[7],
            Limits::default()
        ),
        "Ok(I64(7))"
    );
}

fn p(index: usize) -> Template {
    Template::Param { index }
}
fn binary(kind: Binary, left: Template, right: Template) -> Template {
    Template::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    }
}
fn definition(
    word: &str,
    params: Vec<ScalarType>,
    result: ScalarType,
    body: Template,
) -> Definition {
    Definition {
        word: word.into(),
        params,
        result,
        body,
    }
}

#[test]
fn custom_dag_dictionary_is_versioned_typed_deterministic_and_bound() {
    use ScalarType::Int;
    let first = definition("grow", vec![Int], Int, binary(Binary::Mul, p(0), p(0)));
    let second = definition(
        "tree",
        vec![Int],
        Int,
        Template::Use {
            word: "grow".into(),
            args: vec![Template::Use {
                word: "grow".into(),
                args: vec![p(0)],
            }],
        },
    );
    let dictionary = Dictionary::new(&[first.clone(), second.clone()]).unwrap();
    let restored = Dictionary::from_json(&dictionary.to_json()).unwrap();
    assert_eq!(dictionary.identity(), restored.identity());
    assert_eq!(dictionary.to_json(), restored.to_json());
    let source = format!(
        "wk1 {} v2 pub calc 1 x int int ret tree x",
        dictionary.header()
    );
    let compiled = wordcode::compile(&source, &dictionary, ExpansionLimits::default()).unwrap();
    assert_eq!(
        run(&compiled.module, &[3], Limits::default()),
        "Ok(I64(81))"
    );
    assert_eq!(
        compiled.module.encode(),
        Module::compile_agent("fa1 v2 pub calc 1 x int int ret mul mul x x mul x x")
            .unwrap()
            .encode()
    );
    let mut changed = first;
    changed.body = binary(Binary::Add, p(0), p(0));
    let other = Dictionary::new(&[changed, second]).unwrap();
    assert_ne!(dictionary.identity(), other.identity());
    assert!(wordcode::compile(&source, &other, ExpansionLimits::default()).is_err());
    assert!(wordcode::compile(&source, &Dictionary::core(), ExpansionLimits::default()).is_err());
}

#[test]
fn dictionary_rejects_cycles_forward_edges_capture_duplicate_unused_and_reordered_holes() {
    use ScalarType::{Int, Uint};
    let bad = [
        vec![definition(
            "cycle",
            vec![Int],
            Int,
            Template::Use {
                word: "cycle".into(),
                args: vec![p(0)],
            },
        )],
        vec![
            definition(
                "early",
                vec![Int],
                Int,
                Template::Use {
                    word: "later".into(),
                    args: vec![p(0)],
                },
            ),
            definition("later", vec![Int], Int, p(0)),
        ],
        vec![
            definition("copy", vec![Int], Int, p(0)),
            definition("copy", vec![Int], Int, p(0)),
        ],
        vec![definition("local", vec![Int], Int, p(0))],
        vec![definition("capture", vec![Int], Int, p(1))],
        vec![definition("unused", vec![Int, Int], Int, p(0))],
        vec![definition(
            "reverse",
            vec![Int, Int],
            Int,
            binary(Binary::Sub, p(1), p(0)),
        )],
        vec![definition("wrong", vec![Uint], Int, p(0))],
    ];
    for definitions in bad {
        assert!(Dictionary::new(&definitions).is_err());
    }
    for text in [
        r#"{"version":2,"definitions":[]}"#,
        r#"{"version":1,"definitions":[],"effect":"io"}"#,
        r#"{"version":1,"definitions":[{"word":"free","params":["int"],"result":"int","body":{"op":"local","name":"x"}}]}"#,
    ] {
        assert!(Dictionary::from_json(text).is_err());
    }
}

#[test]
fn repeated_holes_reject_eager_expression_duplication_and_scalar_types_are_exact() {
    for expr in [
        "square add x 1",
        "cube call f 1 x",
        "square 1u",
        "square 0x3f800000f",
        "between 0 true 2",
    ] {
        let text = format!("wk1 core v2 fn f 1 x int int ret x pub calc 1 x int int ret {expr}");
        assert!(
            wordcode::compile(&text, &Dictionary::core(), ExpansionLimits::default()).is_err(),
            "{expr}"
        );
    }
    // Linear recipes retain arbitrary expression arguments in left-to-right order.
    let a = wk("wk1 core v2 pub calc 3 x int y int z int int ret ratio add x y z 0");
    let b =
        Module::compile_agent("fa1 v2 pub calc 3 x int y int z int int ret div add add x y z 0")
            .unwrap();
    assert_eq!(a.encode(), b.encode());
    assert_eq!(
        run(&a, &[i64::MAX, 1, 0], Limits::default()),
        run(&b, &[i64::MAX, 1, 0], Limits::default())
    );
}

#[test]
fn expansion_rejects_invalid_policies_and_oversized_source_output_depth_nodes() {
    let text = "wk1 core v2 pub calc 1 x int int ret cube x";
    let dictionary = Dictionary::core();
    let baseline = wordcode::compile(text, &dictionary, ExpansionLimits::default()).unwrap();
    for limits in [
        ExpansionLimits {
            source_bytes: 1,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            expanded_bytes: 1,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            nodes: 1,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            depth: 1,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            depth: 33,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            nodes: 0,
            ..ExpansionLimits::default()
        },
        ExpansionLimits {
            source_bytes: usize::MAX,
            ..ExpansionLimits::default()
        },
    ] {
        assert!(wordcode::compile(text, &dictionary, limits).is_err());
    }
    let exact = ExpansionLimits {
        source_bytes: text.len(),
        expanded_bytes: baseline.expanded_agent.len(),
        nodes: baseline.report.admission_nodes,
        ..ExpansionLimits::default()
    };
    assert!(wordcode::compile(text, &dictionary, exact).is_ok());
    assert!(
        wordcode::compile(
            text,
            &dictionary,
            ExpansionLimits {
                nodes: exact.nodes - 1,
                ..exact
            }
        )
        .is_err()
    );
    assert!(
        wordcode::compile(
            text,
            &dictionary,
            ExpansionLimits {
                expanded_bytes: exact.expanded_bytes - 1,
                ..exact
            }
        )
        .is_err()
    );
    let deep = format!("wk1 core v2 pub calc 1 x int int ret {}x", "up ".repeat(40));
    assert!(wordcode::compile(&deep, &dictionary, ExpansionLimits::default()).is_err());
}

#[test]
fn nested_dictionary_expansion_has_a_hard_global_ceiling() {
    use ScalarType::Int;
    let mut definitions = vec![definition(
        "a",
        vec![Int],
        Int,
        binary(Binary::Mul, p(0), p(0)),
    )];
    for word in [
        "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "n", "o",
    ] {
        let previous = definitions.last().unwrap().word.clone();
        definitions.push(definition(
            word,
            vec![Int],
            Int,
            Template::Use {
                word: previous.clone(),
                args: vec![Template::Use {
                    word: previous,
                    args: vec![p(0)],
                }],
            },
        ));
    }
    assert!(Dictionary::new(&definitions).is_err());
}

#[test]
fn deterministic_arbitrary_text_never_panics_and_report_binds_payload() {
    let dictionary = Dictionary::core();
    let base = "wk1 core v2 pub calc 1 x int int ret up x";
    let compiled = wordcode::compile(base, &dictionary, ExpansionLimits::default()).unwrap();
    assert_eq!(compiled.report.dictionary_sha256, dictionary.identity());
    assert_eq!(compiled.report.source_bytes, base.len());
    assert_eq!(
        compiled.report.source_words,
        base.split_ascii_whitespace().count()
    );
    assert_eq!(
        Module::compile_agent(&compiled.expanded_agent)
            .unwrap()
            .encode(),
        compiled.module.encode()
    );
    let words = [
        "wk1",
        "core",
        "v2",
        "pub",
        "calc",
        "0",
        "int",
        "ret",
        "repeat",
        "next",
        "walk",
        "up",
        "square",
        "true",
        "-1",
        "65536",
        "wk_private_0",
    ];
    let mut state = 0x123456789abcdef0u64;
    for length in 0..400 {
        let mut text = String::new();
        for _ in 0..length % 40 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            text.push_str(words[state as usize % words.len()]);
            text.push(' ');
        }
        assert!(
            std::panic::catch_unwind(|| wordcode::compile(
                &text,
                &dictionary,
                ExpansionLimits::default()
            ))
            .is_ok()
        );
    }
}

#[test]
fn complete_fa1_language_migrates_with_explicit_collision_escapes() {
    for source in [
        include_str!("../examples/fixtures/control_flow.fluxr"),
        include_str!("../examples/fixtures/list_boundary.fluxr"),
        include_str!("../examples/fixtures/shared_host_admission.fluxr"),
        include_str!("../examples/fixtures/variant_payload.fluxr"),
    ] {
        let original = Module::compile_compact(source).unwrap();
        let fa1 = original.agent_text().unwrap();
        // control_flow has a released local named `step`. Migration explicitly
        // escapes it at its two expression sites; declarations/call names keep
        // their spelling. Blindly changing only the profile header must not be
        // treated as a generally lossless migration.
        let projected = fa1
            .replacen("fa1", "wk1 core", 1)
            .replace("add total step", "add total local step")
            .replace(
                "call quotient 2 total step",
                "call quotient 2 total local step",
            );
        let rebuilt = wk(&projected);
        assert_eq!(rebuilt.encode(), original.encode());
    }
}
