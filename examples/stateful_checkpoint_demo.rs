//! Same-binary, file-based save/resume of an independently packaged FluxR effect.
//! This is a deterministic CPU worker demonstration, never an audio callback.
use fluxr::cpu_compute::stream::stateful::checkpoint::CheckpointAdmission;
use fluxr::cpu_compute::stream::stateful::{
    STATEFUL_ABI, StatefulGraph, StatefulSpec, StatefulStream,
};
use fluxr::cpu_compute::stream::{Argument, GRAPH_ABI, GraphSpec, Node, StreamLimits};
use fluxr::cpu_compute::{Cancellation, MemoryBudget, ScalarLibrary, Tensor};
use fluxr::structured::Limits;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

#[path = "native/effects/stateful/reference.rs"]
mod reference;
const FRAMES: usize = 4096;
const SPLIT: usize = 2048;
const CHUNK: usize = 256;
const PAYLOAD_BYTES: usize = 1 << 20;
const CHECKPOINT_BYTES: usize = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionFile {
    schema: String,
    checkpoint_sha256: [u8; 32],
    checkpoint_bytes: usize,
    graph_identity: String,
    contract_identity: String,
    source_sha256: String,
    prefix_output_sha256: String,
    prefix_state_bits: Vec<u32>,
    next_sequence: u64,
}
fn read(path: &Path, bound: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(bound as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > bound {
        return Err("checkpoint demo input byte bound exceeded".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn limits() -> StreamLimits {
    StreamLimits {
        invocation: Limits::default(),
        maximum_rows: FRAMES as u64,
        maximum_chunk_rows: CHUNK,
        maximum_attempts: FRAMES / CHUNK,
    }
}
fn node(export: &str, arguments: Vec<Argument>) -> Node {
    Node {
        library: 0,
        export: export.into(),
        arguments,
    }
}
fn input(column: usize) -> Argument {
    Argument::Input { column }
}
fn previous(node: usize) -> Argument {
    Argument::Node { node }
}
fn load(root: &Path) -> Result<(StatefulGraph, String), Box<dyn Error>> {
    let package = root.join("effects/stateful");
    let manifest = read(&package.join("manifest.json"), 16 * 1024)?;
    let source = String::from_utf8(read(&package.join("program.flux"), 256 * 1024)?)?;
    let library = Arc::new(ScalarLibrary::load(&manifest, &source)?);
    let source_hash = library.manifest().source_sha256.clone();
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
        &[library],
    )?;
    Ok((graph, source_hash))
}
fn data() -> Vec<f32> {
    let mut result = Vec::with_capacity(FRAMES * 4);
    for frame in 0..FRAMES {
        result.extend_from_slice(&[
            (frame % 128) as f32 / 64. - 1.,
            if frame < 2001 { 0.125 } else { 0.25 },
            0.03125,
            0.25 + (frame % 512) as f32 / 1024.,
        ]);
    }
    result
}
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}
fn hash(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn rust_rows(data: &[f32], mut state: [f32; 2]) -> (Vec<f32>, [f32; 2]) {
    let mut output = Vec::with_capacity(data.len() / 2);
    for row in data.as_chunks::<4>().0 {
        let row = std::hint::black_box(row);
        let low = reference::smooth(row[0], row[1], state[0]);
        let residual = reference::residual(row[0], low);
        let envelope = reference::envelope(residual, row[2], state[1]);
        output.extend_from_slice(&[reference::gain(low, row[3]), envelope]);
        state = [low, envelope];
    }
    (output, state)
}
fn execute(
    graph: &StatefulGraph,
    stream: &mut StatefulStream<'_>,
    data: &[f32],
    memory: &MemoryBudget,
) -> Result<(Vec<f32>, Vec<f64>), Box<dyn Error>> {
    let mut output = Vec::with_capacity(data.len() / 2);
    let mut raw_chunk_seconds = Vec::new();
    for chunk in data.chunks(CHUNK * 4) {
        let started = Instant::now();
        let input = Tensor::new(&[chunk.len() / 4, 4], chunk, memory)?;
        let result = stream.execute(
            graph.identity(),
            stream.state_pin(),
            stream.next_sequence(),
            &input,
        )?;
        output.extend_from_slice(result.output.values());
        raw_chunk_seconds.push(started.elapsed().as_secs_f64());
    }
    Ok((output, raw_chunk_seconds))
}
fn save(root: &Path, checkpoint_path: &Path, admission_path: &Path) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let (graph, source_sha256) = load(root)?;
    let preparation_seconds = started.elapsed().as_secs_f64();
    let data = data();
    let memory = MemoryBudget::new(PAYLOAD_BYTES);
    let mut stream = graph.stream(limits(), memory.clone(), Cancellation::default())?;
    let (output, chunks) = execute(&graph, &mut stream, &data[..SPLIT * 4], &memory)?;
    let (expected, state) = rust_rows(&data[..SPLIT * 4], [0., 0.]);
    if bits(&output) != bits(&expected) || bits(stream.state()) != bits(&state) {
        return Err("saved FluxR prefix differs from independent Rust bits".into());
    }
    let started = Instant::now();
    let checkpoint = stream.checkpoint(CHECKPOINT_BYTES)?;
    let checkpoint_encode_seconds = started.elapsed().as_secs_f64();
    let admission = AdmissionFile {
        schema: "fluxr-checkpoint-demo-admission-v1".into(),
        checkpoint_sha256: checkpoint.sha256(),
        checkpoint_bytes: checkpoint.bytes().len(),
        graph_identity: graph.identity().into(),
        contract_identity: stream.contract_identity().into(),
        source_sha256,
        prefix_output_sha256: hash(&output),
        prefix_state_bits: bits(stream.state()),
        next_sequence: stream.next_sequence(),
    };
    let started = Instant::now();
    write_new(checkpoint_path, checkpoint.bytes())?;
    write_new(admission_path, &serde_json::to_vec_pretty(&admission)?)?;
    let io_seconds = started.elapsed().as_secs_f64();
    println!(
        "{}",
        json!({
            "schema":"fluxr-checkpoint-demo-save-v1","passed":true,
            "preparation_seconds":preparation_seconds,"raw_chunk_seconds":chunks,
            "checkpoint_encode_seconds":checkpoint_encode_seconds,"file_write_fsync_seconds":io_seconds,
            "checkpoint_bytes":checkpoint.bytes().len(),"checkpoint_sha256":checkpoint.sha256(),
            "graph_identity":graph.identity(),"state_pin":stream.state_pin(),
            "statistics":stream.statistics(),"attempts":stream.attempts(),
            "payload_peak_bytes":memory.peak_bytes(),"metadata_byte_limit":CHECKPOINT_BYTES,
            "admission_is_external_to_checkpoint":true,"real_time_callback":false
        })
    );
    Ok(())
}
fn resume(
    root: &Path,
    checkpoint_path: &Path,
    admission_path: &Path,
    report_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let (graph, source_sha256) = load(root)?;
    let preparation_seconds = started.elapsed().as_secs_f64();
    let admission: AdmissionFile = serde_json::from_slice(&read(admission_path, 4096)?)?;
    if admission.schema != "fluxr-checkpoint-demo-admission-v1"
        || admission.graph_identity != graph.identity()
        || admission.source_sha256 != source_sha256
    {
        return Err("checkpoint demo admission subject mismatch".into());
    }
    let started = Instant::now();
    let checkpoint = read(checkpoint_path, CHECKPOINT_BYTES)?;
    let file_read_seconds = started.elapsed().as_secs_f64();
    if checkpoint.len() != admission.checkpoint_bytes {
        return Err("checkpoint demo admitted byte count mismatch".into());
    }
    let memory = MemoryBudget::new(PAYLOAD_BYTES);
    let started = Instant::now();
    let mut resumed = graph.resume_checkpoint(
        &checkpoint,
        CheckpointAdmission {
            maximum_bytes: CHECKPOINT_BYTES,
            expected_sha256: admission.checkpoint_sha256,
        },
        limits(),
        memory.clone(),
        Cancellation::default(),
    )?;
    let checkpoint_resume_seconds = started.elapsed().as_secs_f64();
    if resumed.contract_identity() != admission.contract_identity
        || bits(resumed.state()) != admission.prefix_state_bits
        || resumed.next_sequence() != admission.next_sequence
    {
        return Err("checkpoint demo restored prefix differs from admission".into());
    }
    let data = data();
    let (prefix, state) = rust_rows(&data[..SPLIT * 4], [0., 0.]);
    if hash(&prefix) != admission.prefix_output_sha256
        || bits(&state) != admission.prefix_state_bits
    {
        return Err("saved prefix differs from independent Rust oracle".into());
    }
    let started = Instant::now();
    let (expected, final_state) = rust_rows(&data[SPLIT * 4..], state);
    let rust_continuation_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let (output, chunks) = execute(&graph, &mut resumed, &data[SPLIT * 4..], &memory)?;
    let fluxr_continuation_seconds = started.elapsed().as_secs_f64();
    if bits(&output) != bits(&expected) || bits(resumed.state()) != bits(&final_state) {
        return Err("resumed FluxR differs from independent Rust continuation".into());
    }
    let baseline_memory = MemoryBudget::new(PAYLOAD_BYTES);
    let mut baseline = graph.stream(limits(), baseline_memory.clone(), Cancellation::default())?;
    let (whole, _) = execute(&graph, &mut baseline, &data, &baseline_memory)?;
    if bits(&whole[SPLIT * 2..]) != bits(&output)
        || baseline.state_pin() != resumed.state_pin()
        || serde_json::to_value(baseline.attempts())? != serde_json::to_value(resumed.attempts())?
        || serde_json::to_value(baseline.statistics())?
            != serde_json::to_value(resumed.statistics())?
    {
        return Err("resumed stream differs from uninterrupted full ledger".into());
    }
    let finished = resumed.checkpoint(CHECKPOINT_BYTES)?;
    let uninterrupted = baseline.checkpoint(CHECKPOINT_BYTES)?;
    if finished.bytes() != uninterrupted.bytes() {
        return Err("repeated endpoint checkpoint bytes differ".into());
    }
    let peak = memory.peak_bytes();
    let statistics = serde_json::to_value(resumed.statistics())?;
    let attempts = serde_json::to_value(resumed.attempts())?;
    let pin = resumed.state_pin();
    drop(resumed);
    if memory.live_bytes() != 0 {
        return Err("resumed stream retained tensor payload after teardown".into());
    }
    let report = json!({
        "schema":"fluxr-checkpoint-demo-resume-v1","passed":true,
        "graph_identity":graph.identity(),"source_sha256":source_sha256,
        "input_checkpoint_sha256":admission.checkpoint_sha256,
        "input_checkpoint_bytes":checkpoint.len(),"final_checkpoint_sha256":finished.sha256(),
        "final_checkpoint_bytes":finished.bytes().len(),"complete_checkpoint_bytes_equal":true,
        "frames":FRAMES,"prefix_frames":SPLIT,"continuation_frames":FRAMES-SPLIT,"chunk_rows":CHUNK,
        "state_pin":pin,"final_state_bits":bits(&final_state),
        "continuation_output_sha256":hash(&output),"uninterrupted_output_sha256":hash(&whole),
        "statistics":statistics,"attempts":attempts,"complete_ledger_equal":true,
        "preparation_seconds":preparation_seconds,"checkpoint_file_read_seconds":file_read_seconds,
        "checkpoint_resume_seconds":checkpoint_resume_seconds,
        "rust_continuation_seconds":rust_continuation_seconds,
        "fluxr_continuation_seconds":fluxr_continuation_seconds,
        "fluxr_over_rust_continuation_ratio":fluxr_continuation_seconds/rust_continuation_seconds,
        "raw_chunk_seconds":chunks,"payload_peak_bytes":peak,"payload_live_after_drop":memory.live_bytes(),
        "checkpoint_metadata_limit_bytes":CHECKPOINT_BYTES,"real_time_callback":false,
        "timing_scope":"Same continuation rows, recurrence and appended output vectors. FluxR additionally constructs tensors and checks/hashes complete receipts; checkpoint preparation, file I/O and resume validation are separate.",
        "limits":["CPU worker/offline; no hardware callback, GPU or foreign-native ABI",
          "Checkpoint metadata, code, VM capacity, allocator and host vectors are outside tensor payload accounting; enclosing runner measures process RSS",
          "External expected digest identifies admitted bytes, not truthful execution, authorization, freshness authority or distributed exactly-once delivery",
          "Same checkpoint may deliberately fork; no live graph replacement or incompatible policy migration"]
    });
    write_new(report_path, &serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"schema":"fluxr-checkpoint-demo-resume-v1","passed":true,"report":report_path})
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [mode, root, checkpoint, admission] if mode == "save" => {
            save(Path::new(root), Path::new(checkpoint), Path::new(admission))
        }
        [mode, root, checkpoint, admission, report] if mode == "resume" => {
            resume(Path::new(root), Path::new(checkpoint), Path::new(admission), Path::new(report))
        }
        _ => Err("usage: stateful_checkpoint_demo save ROOT CHECKPOINT ADMISSION_JSON | resume ROOT CHECKPOINT ADMISSION_JSON REPORT_JSON".into()),
    }
}
