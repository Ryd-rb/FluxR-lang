//! Reviewed externally loaded FluxR recurrent DSP fixture. All recurrence,
//! envelope and gain arithmetic executes through ordinary FluxR exports.
use fluxr::cpu_compute::stream::stateful::{STATEFUL_ABI, StatefulGraph, StatefulSpec};
use fluxr::cpu_compute::stream::{Argument, GRAPH_ABI, GraphSpec, Node, StreamLimits};
use fluxr::cpu_compute::{Cancellation, MemoryBudget, ScalarLibrary, Tensor};
use fluxr::structured::Limits;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{error::Error, fs, io::Read, path::Path, sync::Arc, time::Instant};

#[path = "native/effects/stateful/reference.rs"]
mod reference;

fn read(path: &Path, bound: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(bound as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > bound {
        return Err("stateful fixture byte bound exceeded".into());
    }
    Ok(bytes)
}
fn input(column: usize) -> Argument {
    Argument::Input { column }
}
fn previous(node: usize) -> Argument {
    Argument::Node { node }
}
fn node(export: &str, arguments: Vec<Argument>) -> Node {
    Node {
        library: 0,
        export: export.into(),
        arguments,
    }
}
fn hash_values(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).ceil() as usize]
}
fn run(root: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let started = Instant::now();
    let package = root.join("effects/stateful");
    let manifest = read(&package.join("manifest.json"), 16 * 1024)?;
    let source = String::from_utf8(read(&package.join("program.flux"), 256 * 1024)?)?;
    let library = Arc::new(ScalarLibrary::load(&manifest, &source)?);
    let graph = StatefulGraph::prepare(
        StatefulSpec {
            abi: STATEFUL_ABI.into(),
            external_columns: 4,
            transition: GraphSpec {
                abi: GRAPH_ABI.into(),
                input_columns: 6,
                nodes: vec![
                    node("smooth", vec![input(0), input(1), input(4)]),
                    node("residual", vec![input(0), previous(0)]),
                    node("envelope", vec![previous(1), input(2), input(5)]),
                    node("gain", vec![previous(0), input(3)]),
                ],
                outputs: vec![3, 2],
            },
            initial_state_bits: vec![0, 0],
            state_outputs: vec![0, 2],
        },
        std::slice::from_ref(&library),
    )?;
    let preparation_seconds = started.elapsed().as_secs_f64();
    let frames = 24000usize;
    let sample_rate = 24000usize;
    let started = Instant::now();
    let mut data = Vec::with_capacity(frames * 4);
    for frame in 0..frames {
        // Exact dyadic saw input and frame-indexed control changes. No host
        // transcendental approximation is mixed into the recurrence oracle.
        let sample = (frame % 128) as f32 / 64. - 1.;
        let alpha = if frame < 7999 {
            0.0625
        } else if frame < 16001 {
            0.25
        } else {
            0.125
        };
        let envelope_alpha = 0.03125;
        let gain = 0.25 + (frame % 512) as f32 / 1024.;
        data.extend_from_slice(&[sample, alpha, envelope_alpha, gain]);
    }
    let fixture_preparation_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let mut rust_output = Vec::with_capacity(frames * 2);
    let mut state = [0f32; 2];
    for row in data.as_chunks::<4>().0 {
        let row = std::hint::black_box(row);
        let low = reference::smooth(row[0], row[1], state[0]);
        let residual = reference::residual(row[0], low);
        let envelope = reference::envelope(residual, row[2], state[1]);
        rust_output.extend_from_slice(&[reference::gain(low, row[3]), envelope]);
        state = [low, envelope];
    }
    let rust_seconds = started.elapsed().as_secs_f64();
    let reference_hash = hash_values(&rust_output);
    let reference_final_state_bits = state.map(f32::to_bits);
    let started = Instant::now();
    let mut direct = library.session(
        Limits::default(),
        (frames * 4) as u64,
        Cancellation::default(),
    );
    let mut direct_output = Vec::with_capacity(frames * 2);
    let mut state = [0f32; 2];
    for row in data.as_chunks::<4>().0 {
        let low = direct.call("smooth", &[row[0], row[1], state[0]])?;
        let residual = direct.call("residual", &[row[0], low])?;
        let envelope = direct.call("envelope", &[residual, row[2], state[1]])?;
        let gained = direct.call("gain", &[low, row[3]])?;
        direct_output.extend_from_slice(&[gained, envelope]);
        state = [low, envelope];
    }
    let direct_seconds = started.elapsed().as_secs_f64();
    if hash_values(&direct_output) != reference_hash
        || state.map(f32::to_bits) != reference_final_state_bits
    {
        return Err("direct FluxR recurrence differs from strict Rust bits".into());
    }
    let mut observations = Vec::new();
    for chunk_rows in [32usize, 256, 1024] {
        let memory = MemoryBudget::new(64 * 1024);
        let started = Instant::now();
        let mut stream = graph.stream(
            StreamLimits {
                invocation: Limits::default(),
                maximum_rows: frames as u64,
                maximum_chunk_rows: chunk_rows,
                maximum_attempts: frames.div_ceil(chunk_rows),
            },
            memory.clone(),
            Cancellation::default(),
        )?;
        let stream_preparation_seconds = started.elapsed().as_secs_f64();
        let mut output = Vec::with_capacity(frames * 2);
        let mut input_seconds = 0.;
        let mut execution_seconds = 0.;
        let mut raw_chunk_us = Vec::new();
        let mut simulated_deadline_misses = 0usize;
        for chunk in data.chunks(chunk_rows * 4) {
            let count = chunk.len() / 4;
            let started = Instant::now();
            let input = Tensor::new(&[count, 4], chunk, &memory)?;
            input_seconds += started.elapsed().as_secs_f64();
            let started = Instant::now();
            let result = stream.execute(
                graph.identity(),
                stream.state_pin(),
                stream.next_sequence(),
                &input,
            )?;
            let seconds = started.elapsed().as_secs_f64();
            execution_seconds += seconds;
            raw_chunk_us.push(seconds * 1e6);
            simulated_deadline_misses += usize::from(seconds > count as f64 / sample_rate as f64);
            output.extend_from_slice(result.output.values());
        }
        if hash_values(&output) != reference_hash
            || stream
                .state()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
                != reference_final_state_bits
        {
            return Err(
                "stateful FluxR output or carried state differs from reference or chunking".into(),
            );
        }
        let observation = json!({"chunk_rows":chunk_rows,"frames":frames,"stream_preparation_seconds":stream_preparation_seconds,
            "input_tensor_seconds":input_seconds,"execution_seconds":execution_seconds,"frames_per_second":frames as f64/execution_seconds,
            "chunk_us":{"p50":quantile(&raw_chunk_us,0.5),"p95":quantile(&raw_chunk_us,0.95),"p99":quantile(&raw_chunk_us,0.99),"max":quantile(&raw_chunk_us,1.)},
            "raw_chunk_us":raw_chunk_us,"simulated_deadline_misses":simulated_deadline_misses,"actual_callback_executed":false,
            "output_value_bits_sha256":hash_values(&output),"final_state_bits":stream.state().iter().map(|v|v.to_bits()).collect::<Vec<_>>(),
            "state_pin":stream.state_pin(),"contract":stream.contract(),"contract_identity":stream.contract_identity(),
            "statistics":stream.statistics(),"attempts":stream.attempts(),"payload_peak_bytes":memory.peak_bytes()});
        drop(stream);
        if memory.live_bytes() != 0 {
            return Err("stateful stream retained payload after teardown".into());
        }
        observations.push(observation);
    }
    Ok(
        json!({"schema":"fluxr-stateful-stream-demo-v1","passed":true,"graph_identity":graph.identity(),
        "graph_spec":serde_json::from_slice::<serde_json::Value>(&graph.encode())?,"library_identity":library.identity(),
        "package_source_sha256":library.manifest().source_sha256,"backend":"released FluxR portable CPU evaluator",
        "preparation_seconds":preparation_seconds,"fixture_preparation_seconds":fixture_preparation_seconds,
        "frames":frames,"sample_rate":sample_rate,"duration_seconds":1.,"state_cells":2,
        "reference_value_bits_sha256":reference_hash,"reference_final_state_bits":reference_final_state_bits,
        "rust_execution_seconds":rust_seconds,"direct_fluxr_execution_seconds":direct_seconds,"direct_fluxr_over_rust_ratio":direct_seconds/rust_seconds,
        "reference_timing_scope":"same input arrays, exact recurrence/envelope/gain operations, output Vec append; hashes excluded; direct FluxR includes session preparation; graph additionally includes receipt/state/tensor hashing and admission",
        "sample_automation":"explicit per-frame columns; low-pass coefficient changes at frames 7999 and 16001; gain changes each frame",
        "observations":observations,"limitations":["CPU offline/worker path; no audio device callback, physical GPU or external native plugin executed",
            "One-second deterministic DSP fixture extends but does not replace inherited multitrack composition",
            "Payload quota excludes code, VM capacities, metadata, allocator overhead and host oracle/fixture arrays",
            "Revision/digest pins are local to live stream handles, not crash-persistent authorization or distributed exactly-once delivery",
            "New stateful graph and receipts have explicit ABI; released stateless graph meanings are unchanged"]}),
    )
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: stateful_stream_demo NATIVE_FIXTURE_ROOT OUTPUT_JSON".into());
    }
    let report = run(Path::new(&args[0]))?;
    fs::write(&args[1], serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"schema":"fluxr-stateful-stream-demo-v1","passed":true,"output":args[1]})
    );
    Ok(())
}
