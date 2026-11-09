//! Timing and deterministic trace output for a state transition export.
use crate::structured::{Invocation, Limits, Module, Value, Workspace};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{error::Error, io::Write, time::Instant};

const USAGE: &str =
    "expected <time|trace> <start> <step> <inspect> <warmup> <samples> [integer ...]";
const LIMITS: Limits = Limits {
    instructions: 20_000_000,
    cells: 8_000_000,
    frames: 64,
};

fn hash(state: &Value) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(state)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Execute a bounded local series using exports named by the caller.
pub fn run(
    module: &Module,
    mut args: impl Iterator<Item = String>,
    mut output: impl Write,
) -> Result<(), Box<dyn Error>> {
    let trace = match args.next().as_deref() {
        Some("trace") => true,
        #[cfg(target_arch = "wasm32")]
        Some("time") => return Err("series timing requires a native host".into()),
        #[cfg(not(target_arch = "wasm32"))]
        Some("time") => false,
        _ => return Err(USAGE.into()),
    };
    let start = args.next().ok_or(USAGE)?;
    let step = args.next().ok_or(USAGE)?;
    let inspect = args.next().ok_or(USAGE)?;
    let warmup: usize = args.next().ok_or(USAGE)?.parse()?;
    let samples: usize = args.next().ok_or(USAGE)?.parse()?;
    if warmup > 1_024 || !(1..=1_024).contains(&samples) {
        return Err(USAGE.into());
    }
    let arguments = args
        .map(|arg| arg.parse::<i64>().map(Value::I64))
        .collect::<Result<Vec<_>, _>>()?;
    let mut workspace = Workspace::default();
    let mut invoke = |export: &str, arguments: &[Value]| {
        module.run_with(
            Invocation {
                export,
                inputs: &[],
                arguments,
                limits: LIMITS,
            },
            &mut workspace,
        )
    };

    let mut state = invoke(&start, &arguments)?;
    let mut timings = Vec::with_capacity(samples);
    for tick in 1..=warmup + samples {
        let started = (!trace && tick > warmup).then(Instant::now);
        state = invoke(&step, std::slice::from_ref(&state))?;
        if let Some(started) = started {
            timings.push(started.elapsed().as_nanos() as u64);
        }
        if trace {
            writeln!(
                output,
                "{}",
                json!({
                    "tick": tick,
                    "sha256": hash(&state)?,
                    "status": invoke(&inspect, std::slice::from_ref(&state))?,
                }),
            )?;
        }
    }
    if !trace {
        writeln!(
            output,
            "{}",
            json!({
                "warmup": warmup,
                "tick_ns": timings,
                "sha256": hash(&state)?,
                "status": invoke(&inspect, std::slice::from_ref(&state))?,
            }),
        )?;
    }
    Ok(())
}
