use fluxr::{Program, telemetry};
use std::{fmt::Write, hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const TRACKS: u16 = 128;
    const FRAMES: u32 = 50_000;
    let mut script = String::from("v1 1000");
    for id in 0..TRACKS {
        write!(script, "\n{id} 1 1 0:{id} 500:{} 1000:{id}", id + 500)?;
    }
    let program = Program::compile_text(&script)?;
    let measure = |sample: &dyn Fn(u32) -> i64| {
        let started = Instant::now();
        let checksum: i64 = (0..FRAMES)
            .map(|tick| black_box(sample(black_box(tick))))
            .sum();
        (
            checksum,
            started.elapsed().as_nanos() as f64 / f64::from(FRAMES * u32::from(TRACKS)),
        )
    };
    let (direct, direct_ns) = measure(&|time| {
        let t = (time % 1000) as i32;
        let height = t.min(1000 - t);
        (0..TRACKS)
            .map(|id| i64::from(black_box(i32::from(id) + height)))
            .sum()
    });
    let (scripted, scripted_ns) = measure(&|time| {
        program
            .sample_all(time)
            .map(|value| i64::from(black_box(value)))
            .sum()
    });
    assert_eq!(direct, scripted);
    println!(
        "{FRAMES} frames × {TRACKS} tracks, same checksum {scripted}, direct Rust {direct_ns:.2} ns/sample, FluxR {scripted_ns:.2} ns/sample"
    );

    let mut disabled = Vec::new();
    let mut enabled = Vec::new();
    for _ in 0..5 {
        for (mask, results) in [("off", &mut disabled), ("sample-batch", &mut enabled)] {
            telemetry::configure(mask)?;
            let (sum, ns) = measure(&|time| {
                program
                    .sample_all(time)
                    .map(|value| i64::from(black_box(value)))
                    .sum()
            });
            assert_eq!(sum, direct);
            results.push(ns);
        }
    }
    disabled.sort_by(f64::total_cmp);
    enabled.sort_by(f64::total_cmp);
    println!(
        "FluxR batch telemetry median over 5 passes: off {:.2} ns/sample, on {:.2} ns/sample (aggregate counter updates per batch of {TRACKS})",
        disabled[2], enabled[2]
    );
    Ok(())
}
