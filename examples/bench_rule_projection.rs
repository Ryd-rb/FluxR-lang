//! Compare warm signed rule projection with scalar and legacy release controls.
#[path = "fixtures/rule_projection_checks.rs"]
mod checks;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn rule_projection_check() -> usize {
    checks::check()
}

#[test]
fn signed_projection_matches_rule_and_rust() {
    assert_eq!(checks::check(), 16_195);
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use fluxr::RuleProgram;
    use fluxr::hir::Hir;
    use fluxr::structured::{Limits, Value, Workspace};
    use std::{hint::black_box, time::Instant};

    assert_eq!(checks::check(), 16_195);
    let reverse = std::env::args().any(|arg| arg == "reverse");
    let rule = RuleProgram::compile_text(checks::AND).unwrap();
    let hir = Hir::from_rule(rule.clone()).unwrap();
    let projection = hir.prepare_rule_projection().unwrap();
    let control = Hir::from_rule(RuleProgram::compile_text(checks::ADD).unwrap()).unwrap();
    let control = control.prepare_rule_projection().unwrap();
    let limits = Limits::default();
    let [mut bitand, mut add] = std::array::from_fn(|_| Workspace::default());
    let prepare = |_: usize| {
        black_box(black_box(&hir).prepare_rule_projection().unwrap());
    };

    println!("case,route,round,ns_per_operation");
    for (case, factor) in [("mixed_sign", -1i64), ("same_sign", 1)] {
        let pairs: [[i64; 2]; 256] =
            std::array::from_fn(|i| [i as i64 * 29 - 73, (i as i64 * 13 - 67) * factor]);
        let args = pairs.map(|pair| pair.map(Value::I64));
        for (&[a, b], args) in pairs.iter().zip(&args) {
            assert_eq!(rule.evaluate(&[a, b]), Ok(a & b));
            assert_eq!(
                projection.run_with(args, limits, &mut bitand),
                Ok(Value::I64(a & b))
            );
            assert_eq!(
                control.run_with(args, limits, &mut add),
                Ok(Value::I64(a + b))
            );
        }
        let mut routes: [(&str, &mut dyn FnMut(usize)); 4] = [
            ("projected_and", &mut |i| {
                let _ = black_box(projection.run_with(black_box(&args[i]), limits, &mut bitand));
            }),
            ("structured_add", &mut |i| {
                let _ = black_box(control.run_with(black_box(&args[i]), limits, &mut add));
            }),
            ("legacy_and", &mut |i| {
                let _ = black_box(rule.evaluate(black_box(&pairs[i])));
            }),
            ("rust_and", &mut |i| {
                let [a, b] = *black_box(&pairs[i]);
                black_box(a & b);
            }),
        ];
        for (_, run) in &mut routes {
            for i in 0..4096 {
                run(i % 256);
            }
        }
        for round in 0..21 {
            for step in 0..routes.len() {
                let index = if (round % 2 == 1) ^ reverse {
                    routes.len() - step - 1
                } else {
                    step
                };
                let (label, run) = &mut routes[index];
                let start = Instant::now();
                for i in 0..65_536 {
                    run(i % 256);
                }
                println!(
                    "{case},{label},{round},{:.3}",
                    start.elapsed().as_secs_f64() * 1e9 / 65_536.
                );
            }
        }
    }
    for round in 0..21 {
        let start = Instant::now();
        for i in 0..4096 {
            prepare(i);
        }
        println!(
            "mixed_sign,prepare,{round},{:.3}",
            start.elapsed().as_secs_f64() * 1e9 / 4096.
        );
    }
}
