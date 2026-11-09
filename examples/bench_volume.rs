use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use sha2::{Digest, Sha256};
use std::{error::Error, hint::black_box, time::Instant};

const SAMPLES: usize = 20_000;
const PASSES: usize = 5;
const SOURCE: &str = include_str!("bench_volume.flux");

#[path = "fixtures/bench_volume_host.rs"]
#[allow(
    clippy::needless_late_init,
    clippy::needless_question_mark,
    clippy::too_many_arguments
)]
mod aot;
#[path = "fixtures/bench_volume_reference.rs"]
mod reference;

fn measure(mut evaluate: impl FnMut(usize) -> i64) -> (i64, f64) {
    let started = Instant::now();
    let checksum = (0..SAMPLES)
        .map(|index| black_box(evaluate(black_box(index))))
        .sum();
    (
        checksum,
        started.elapsed().as_nanos() as f64 / SAMPLES as f64,
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let cases: Vec<[i64; 11]> = (0..SAMPLES as i64)
        .map(|sample| {
            std::array::from_fn(|index| {
                let lane = index as i64;
                let value = sample * (23 + 14 * lane) + 43 * lane;
                if index < 8 {
                    value % 4093 - 2046
                } else {
                    value % 1025
                }
            })
        })
        .collect();
    let arguments: Vec<_> = cases.iter().map(|case| case.map(Value::I64)).collect();
    let compile_started = Instant::now();
    let module = Module::compile_compact(SOURCE)?;
    let compile_time = compile_started.elapsed();
    let prepare_started = Instant::now();
    let identity: [u8; 32] = Sha256::digest(module.encode()).into();
    assert_eq!(
        identity,
        aot::IDENTITY,
        "regenerate the native benchmark fixture"
    );
    let limits = Limits::default();
    let required = module
        .native_ceiling("volume", 11)
        .expect("bounded native export");
    assert!(limits.instructions >= required.instructions);
    assert!(limits.cells >= required.cells);
    assert!(limits.frames >= required.frames);
    let prepare_time = prepare_started.elapsed();

    for overflow in [
        [i64::MIN, i64::MAX, 0, 0, 0, 0, 0, 0, 1, 0, 0],
        [0, i64::MAX, 0, 0, 0, 0, 0, 0, 2, 0, 0],
    ] {
        assert_eq!(aot::run(overflow), None);
        assert_eq!(reference::volume(overflow), None);
        assert!(
            module
                .run(Invocation {
                    export: "volume",
                    inputs: &[],
                    arguments: &overflow.map(Value::I64),
                    limits,
                })
                .is_err()
        );
    }
    let mut workspace = Workspace::default();
    let mut fluxr = |index: usize| {
        let result = module
            .run_with(
                Invocation {
                    export: "volume",
                    inputs: &[],
                    arguments: &arguments[index],
                    limits,
                },
                &mut workspace,
            )
            .expect("valid interpolation invocation");
        let Value::I64(value) = result else {
            panic!("interpolation returned an unexpected type");
        };
        value
    };
    for (index, &case) in cases.iter().enumerate() {
        let expected = reference::volume(case).unwrap();
        assert_eq!(expected, fluxr(index), "VM sample {index}");
        assert_eq!(Some(expected), aot::run(case), "AOT sample {index}");
    }

    let mut direct_times = [0.0; PASSES];
    let mut fluxr_times = [0.0; PASSES];
    let mut aot_times = [0.0; PASSES];
    for pass in 0..PASSES {
        let (direct, direct_ns) = measure(|index| reference::volume(cases[index]).unwrap());
        let (compiled, compiled_ns) = measure(|index| aot::run(cases[index]).unwrap());
        let (scripted, scripted_ns) = measure(&mut fluxr);
        assert_eq!(direct, compiled);
        assert_eq!(direct, scripted);
        direct_times[pass] = direct_ns;
        fluxr_times[pass] = scripted_ns;
        aot_times[pass] = compiled_ns;
    }
    direct_times.sort_by(f64::total_cmp);
    fluxr_times.sort_by(f64::total_cmp);
    aot_times.sort_by(f64::total_cmp);
    let direct = direct_times[PASSES / 2];
    let scripted = fluxr_times[PASSES / 2];
    let compiled = aot_times[PASSES / 2];
    println!("{SAMPLES} samples × {PASSES} passes, exact parity for every sample");
    println!("Compile: {compile_time:?}, native admission: {prepare_time:?}");
    println!(
        "Median: Rust {direct:.2} ns/sample, FluxR AOT {compiled:.2} ns/sample, FluxR VM {scripted:.2} ns/sample"
    );
    Ok(())
}
