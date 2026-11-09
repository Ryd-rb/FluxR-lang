//! Compare raw binary32 rule execution with typed, legacy, and Rust controls.
#[path = "fixtures/raw_rule_projection_checks.rs"]
mod checks;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "fixtures/composed_rule_hir_checks.rs"]
mod composed;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "fixtures/rule_origin_artifact_checks.rs"]
mod rule_origin_artifact;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn raw_rule_projection_check() -> usize {
    checks::check()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn composed_rule_hir_check() -> usize {
    composed::check()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn rule_limit_hir_check() -> usize {
    composed::check_limits()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn rule_origin_artifact_safety_check() -> usize {
    rule_origin_artifact::check()
}

#[test]
fn rule_origin_artifacts_match_frozen_refusals() {
    assert_eq!(rule_origin_artifact::check(), 25);
}

#[test]
fn composed_rule_hir_matches_frozen_ledger() {
    assert_eq!(composed::check(), 28);
}

#[test]
fn rule_limits_match_frozen_ledger() {
    assert_eq!(composed::check_limits(), 18);
}

#[test]
fn raw_projection_matches_rule_and_rust() {
    assert_eq!(checks::check(), 68_712);
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use checks::{ADD, CHAIN, DIV, EDGES, EXACT, LE, as_float, as_word, chain};
    use fluxr::RuleProgram;
    use fluxr::hir::Hir;
    use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
    use std::{hint::black_box, time::Instant};

    const COUNT: usize = 256;
    const ROUNDS: usize = 21;
    const OPERATIONS: usize = 65_536;
    let reverse = std::env::args().any(|arg| arg == "reverse");
    type Kernel = fn([i64; 3]) -> i64;
    let cases: [(&str, &str, Option<&str>, Kernel); 4] = [
        (
            "fadd",
            ADD,
            Some(
                "v1 (pub rule (a:f32 b:f32) f32 (x:f32 y:f32) (= x a) (= y b) (= x (+ x y)) (ret x))",
            ),
            |[a, b, _]| as_word(as_float(a) + as_float(b)),
        ),
        (
            "fdiv",
            DIV,
            Some(
                "v1 (pub rule (a:f32 b:f32) f32 (x:f32 y:f32) (= x a) (= y b) (= x (/ x y)) (ret x))",
            ),
            |[a, b, _]| as_word(as_float(a) / as_float(b)),
        ),
        (
            "fle",
            LE,
            Some(
                "v1 (pub rule (a:f32 b:f32) bool (x:f32 y:bool) (= x a) (= b b) (= y (<= x b)) (ret y))",
            ),
            |[a, b, _]| i64::from(as_float(a) <= as_float(b)),
        ),
        ("mixed", CHAIN, None, chain),
    ];
    println!("case,distribution,route,round,ns_per_operation");
    for (case, text, control, kernel) in cases {
        let legacy = RuleProgram::compile_text(text).unwrap();
        let hir = Hir::from_rule(legacy.clone()).unwrap();
        let projected = hir.prepare_rule_projection().unwrap();
        let typed = control.map(|source| Module::compile_compact(source).unwrap());
        let n = if typed.is_some() { 2 } else { 3 };
        let limits = if n == 2 { EXACT } else { Limits::default() };
        let [mut projection_space, mut typed_space] = std::array::from_fn(|_| Workspace::default());
        for distribution in ["finite", "special"] {
            let inputs: [[i64; 3]; COUNT] = std::array::from_fn(|i| {
                let bits = if distribution == "finite" {
                    [
                        ((i as f32 - 127.5) / 13.0).to_bits(),
                        ((i as f32 + 1.5) / 31.0).to_bits(),
                    ]
                } else {
                    [
                        EDGES[i % EDGES.len()],
                        EDGES[(i / EDGES.len() + 3 * i) % EDGES.len()],
                    ]
                };
                [
                    ((i as i64 - 128) << 32) | i64::from(bits[0]),
                    (0x5a5a_0000_i64 << 32) | i64::from(bits[1]),
                    i64::from(0x7fff_ffff_u32.rotate_left(i as u32)),
                ]
            });
            let signed_args = inputs.map(|row| row.map(Value::I64));
            let typed_args = inputs.map(|[a, b, _]| [Value::F32(a as u32), Value::F32(b as u32)]);
            let mut invoke_typed = |i: usize| {
                typed.as_ref().unwrap().run_portable_with(
                    Invocation {
                        export: "rule",
                        inputs: &[],
                        arguments: black_box(typed_args[i].as_slice()),
                        limits,
                    },
                    &mut typed_space,
                )
            };
            for (i, &row) in inputs.iter().enumerate() {
                let expected = kernel(row);
                assert_eq!(legacy.evaluate(&row[..n]), Ok(expected));
                assert_eq!(
                    projected.run_with(&signed_args[i][..n], limits, &mut projection_space),
                    Ok(Value::I64(expected))
                );
                if typed.is_some() {
                    let word = match invoke_typed(i).unwrap() {
                        Value::F32(bits) => i64::from(bits),
                        Value::Bool(result) => i64::from(result),
                        value => panic!("unexpected control result {value:?}"),
                    };
                    assert_eq!(word, expected);
                }
            }
            let mut run_projected = |i: usize| {
                let _ = black_box(projected.run_with(
                    black_box(&signed_args[i][..n]),
                    limits,
                    &mut projection_space,
                ));
            };
            let mut run_typed = |i| {
                let _ = black_box(invoke_typed(i));
            };
            let mut run_legacy = |i: usize| {
                let _ = black_box(legacy.evaluate(black_box(&inputs[i][..n])));
            };
            let mut run_rust = |i: usize| {
                black_box(kernel(*black_box(&inputs[i])));
            };
            let mut routes: Vec<(&str, &mut dyn FnMut(usize))> = vec![
                ("projected", &mut run_projected),
                ("legacy", &mut run_legacy),
                ("rust", &mut run_rust),
            ];
            if typed.is_some() {
                routes.push(("typed", &mut run_typed));
            }
            for (_, run) in &mut routes {
                for i in 0..4096 {
                    run(i % COUNT);
                }
            }
            for round in 0..ROUNDS {
                for step in 0..routes.len() {
                    let index = if (round % 2 == 1) ^ reverse {
                        routes.len() - step - 1
                    } else {
                        step
                    };
                    let (label, run) = &mut routes[index];
                    let start = Instant::now();
                    for i in 0..OPERATIONS {
                        run(i % COUNT);
                    }
                    println!(
                        "{case},{distribution},{label},{round},{:.3}",
                        start.elapsed().as_secs_f64() * 1e9 / OPERATIONS as f64
                    );
                }
            }
        }
        for round in 0..ROUNDS {
            let start = Instant::now();
            for _ in 0..4096 {
                black_box(black_box(&hir).prepare_rule_projection().unwrap());
            }
            println!(
                "{case},none,prepare,{round},{:.3}",
                start.elapsed().as_secs_f64() * 1e9 / 4096.0
            );
        }
    }
}
