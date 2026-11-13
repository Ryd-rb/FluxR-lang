//! Execute externally packaged FluxR preprocessing, models, instruments and
//! effects through the checked CPU stream graph. This reviewed-fixture runner
//! is not an arbitrary-candidate sandbox or a real-time audio callback.
use fluxr::cpu_compute::stream::{Argument, GRAPH_ABI, GraphSpec, Node, ScalarGraph, StreamLimits};
use fluxr::cpu_compute::{Cancellation, MemoryBudget, ModelCheckpoint, ScalarLibrary, Tensor};
use fluxr::structured::Limits;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{error::Error, fs, io::Read, path::Path, sync::Arc, time::Instant};

fn read(path: &Path, limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("native stream fixture byte bound exceeded".into());
    }
    Ok(bytes)
}
fn library(root: &Path) -> Result<Arc<ScalarLibrary>, Box<dyn Error>> {
    let manifest = read(&root.join("manifest.json"), 16 * 1024)?;
    let source = String::from_utf8(read(&root.join("program.flux"), 256 * 1024)?)?;
    Ok(Arc::new(ScalarLibrary::load(&manifest, &source)?))
}
fn input(column: usize) -> Argument {
    Argument::Input { column }
}
fn constant(value: f32) -> Argument {
    Argument::Constant {
        bits: value.to_bits(),
    }
}
fn previous(node: usize) -> Argument {
    Argument::Node { node }
}
fn node(library: usize, export: &str, arguments: Vec<Argument>) -> Node {
    Node {
        library,
        export: export.into(),
        arguments,
    }
}
fn add_values(digest: &mut Sha256, values: &[f32]) {
    for value in values {
        digest.update(value.to_bits().to_le_bytes());
    }
}
fn x_at(row: usize) -> f32 {
    (row as i32 - 4096) as f32 / 1024.
}
fn ml_reference(x: f32, p: &[f32]) -> [f32; 2] {
    let scaled = x * 0.5 + 0.25;
    let hidden = p[0] * scaled + p[1];
    let hidden = if hidden < 0. { 0. } else { hidden };
    let prediction = hidden * p[2] + p[3];
    let a = scaled - 2.;
    let b = scaled - 0.;
    let cluster = if a * a < b * b { 1. } else { 0. };
    [prediction, cluster]
}
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).ceil() as usize]
}
fn ml(root: &Path) -> Result<Value, Box<dyn Error>> {
    let start = Instant::now();
    let library = library(&root.join("ml"))?;
    let imported = read(&root.join("ml/pretrained.json"), 64 * 1024)?;
    let checkpoint = ModelCheckpoint::decode(&imported, &library, 4)?;
    let p = checkpoint.parameters();
    let spec = GraphSpec {
        abi: GRAPH_ABI.into(),
        input_columns: 1,
        nodes: vec![
            node(0, "affine", vec![input(0), constant(0.5), constant(0.25)]),
            node(
                0,
                "predict",
                vec![
                    previous(0),
                    constant(p[0]),
                    constant(p[1]),
                    constant(p[2]),
                    constant(p[3]),
                ],
            ),
            node(0, "cluster", vec![previous(0), constant(0.), constant(2.)]),
        ],
        outputs: vec![1, 2],
    };
    let graph = ScalarGraph::prepare(spec, std::slice::from_ref(&library))?;
    let preparation_seconds = start.elapsed().as_secs_f64();
    let rows = 8192usize;
    let mut reference_digest = Sha256::new();
    let reference_start = Instant::now();
    for row in 0..rows {
        add_values(
            &mut reference_digest,
            &std::hint::black_box(ml_reference(
                std::hint::black_box(x_at(row)),
                std::hint::black_box(&p),
            )),
        );
    }
    let rust_reference_seconds = reference_start.elapsed().as_secs_f64();
    let reference_hash = format!("{:x}", reference_digest.finalize());
    let direct_start = Instant::now();
    let mut direct = library.session(
        Limits::default(),
        (rows * 3) as u64,
        Cancellation::default(),
    );
    let mut direct_digest = Sha256::new();
    for row in 0..rows {
        let x = direct.call("affine", &[x_at(row), 0.5, 0.25])?;
        let predict = direct.call("predict", &[x, p[0], p[1], p[2], p[3]])?;
        let cluster = direct.call("cluster", &[x, 0., 2.])?;
        add_values(&mut direct_digest, &[predict, cluster]);
    }
    let direct_seconds = direct_start.elapsed().as_secs_f64();
    if format!("{:x}", direct_digest.finalize()) != reference_hash {
        return Err("direct FluxR schedule differs from matched Rust bits".into());
    }
    let mut observations = Vec::new();
    for chunk_rows in [32usize, 256, 1024] {
        let memory = MemoryBudget::new(64 * 1024);
        let config = StreamLimits {
            invocation: Limits::default(),
            maximum_rows: rows as u64,
            maximum_chunk_rows: chunk_rows,
            maximum_attempts: rows.div_ceil(chunk_rows),
        };
        let mut stream = graph.stream(config, memory.clone(), Cancellation::default())?;
        let mut digest = Sha256::new();
        let mut input_seconds = 0.;
        let mut execution_seconds = 0.;
        let mut latencies = Vec::new();
        for offset in (0..rows).step_by(chunk_rows) {
            let count = (rows - offset).min(chunk_rows);
            let started = Instant::now();
            let data = (offset..offset + count).map(x_at).collect::<Vec<_>>();
            let input = Tensor::new(&[count, 1], &data, &memory)?;
            input_seconds += started.elapsed().as_secs_f64();
            let started = Instant::now();
            let output = stream.execute(graph.identity(), stream.next_sequence(), &input)?;
            let elapsed = started.elapsed().as_secs_f64();
            execution_seconds += elapsed;
            latencies.push(elapsed * 1e6);
            for (row, values) in output.output.values().as_chunks::<2>().0.iter().enumerate() {
                let expected = ml_reference(x_at(offset + row), &p);
                if values[0].to_bits() != expected[0].to_bits()
                    || values[1].to_bits() != expected[1].to_bits()
                {
                    return Err("stream model bits differ from Rust reference".into());
                }
            }
            add_values(&mut digest, output.output.values());
        }
        let actual_hash = format!("{:x}", digest.finalize());
        if actual_hash != reference_hash {
            return Err("stream output hash depends on chunking".into());
        }
        let contract = serde_json::to_value(stream.contract())?;
        let contract_identity = stream.contract_identity().to_string();
        let statistics = serde_json::to_value(stream.statistics())?;
        let attempts = serde_json::to_value(stream.attempts())?;
        let peak = memory.peak_bytes();
        drop(stream);
        if memory.live_bytes() != 0 {
            return Err("stream retained tensor payload after completion".into());
        }
        observations.push(json!({"chunk_rows":chunk_rows,"rows":rows,"chunks":latencies.len(),"input_preparation_seconds":input_seconds,"execution_seconds":execution_seconds,"rows_per_second":rows as f64/execution_seconds,"chunk_us":{"p50":quantile(&latencies,0.5),"p95":quantile(&latencies,0.95),"p99":quantile(&latencies,0.99),"max":quantile(&latencies,1.)},"raw_chunk_us":latencies,"payload_peak_bytes":peak,"payload_live_after_drop":memory.live_bytes(),"output_value_bits_sha256":actual_hash,"stream_contract":contract,"stream_contract_identity":contract_identity,"statistics":statistics,"attempts":attempts}));
    }
    Ok(
        json!({"schema":"fluxr-native-stream-ml-v1","backend":"released FluxR portable CPU scalar evaluator","graph_identity":graph.identity(),"graph_spec":serde_json::from_slice::<Value>(&graph.encode())?,"library_identity":library.identity(),"source_sha256":library.manifest().source_sha256,"imported_checkpoint_sha256":format!("{:x}",Sha256::digest(&imported)),"import_route":"existing versioned analytical checkpoint; not a learned model zoo or ONNX importer","preparation_seconds":preparation_seconds,"rows":rows,"reference_value_bits_sha256":reference_hash,"rust_reference_seconds":rust_reference_seconds,"direct_fluxr_seconds":direct_seconds,"direct_fluxr_over_rust_ratio":direct_seconds/rust_reference_seconds,"reference_timing_scope":"same preprocessing, prediction, nearest-centroid rule and output-bit hashing; direct FluxR excludes Tensor construction, graph receipts and input/output FTB1 hashing; Rust also excludes interpreter calls","stream_observations":observations,"limitations":["CPU worker/offline route; no GPU execution or real-time callback","Payload includes row scratch and output but excludes code, VM capacities, ledger metadata and allocator overhead","New stream results do not alter retained training overhead or imply general agent superiority"]}),
    )
}
fn effects(root: &Path) -> Result<Value, Box<dyn Error>> {
    let start = Instant::now();
    let triangle = library(&root.join("instruments/triangle"))?;
    let bell = library(&root.join("instruments/bell"))?;
    let effect = library(&root.join("effects/softclip"))?;
    let graph = ScalarGraph::prepare(
        GraphSpec {
            abi: GRAPH_ABI.into(),
            input_columns: 3,
            nodes: vec![
                node(0, "sample", vec![input(0), input(1), input(2)]),
                node(1, "sample", vec![input(0), input(1), input(2)]),
                node(2, "mix", vec![previous(0), previous(1), constant(0.35)]),
                node(2, "softclip", vec![previous(2), constant(1.5)]),
            ],
            outputs: vec![2, 3],
        },
        &[triangle.clone(), bell.clone(), effect.clone()],
    )?;
    let preparation_seconds = start.elapsed().as_secs_f64();
    let rows = 8000usize;
    let chunk_rows = 128usize;
    let memory = MemoryBudget::new(64 * 1024);
    let mut stream = graph.stream(
        StreamLimits {
            invocation: Limits::default(),
            maximum_rows: rows as u64,
            maximum_chunk_rows: chunk_rows,
            maximum_attempts: rows.div_ceil(chunk_rows),
        },
        memory.clone(),
        Cancellation::default(),
    )?;
    let mut direct_triangle =
        triangle.session(Limits::default(), rows as u64, Cancellation::default());
    let mut direct_bell = bell.session(Limits::default(), rows as u64, Cancellation::default());
    let mut dry_digest = Sha256::new();
    let mut wet_digest = Sha256::new();
    let mut execution_seconds = 0.;
    let mut sum_sq = 0f64;
    let mut peak = 0f32;
    for offset in (0..rows).step_by(chunk_rows) {
        let count = (rows - offset).min(chunk_rows);
        let mut values = Vec::with_capacity(count * 3);
        for row in offset..offset + count {
            values.extend_from_slice(&[(row as f32 * 220. / 8000.) % 1., row as f32 / 8000., 0.7]);
        }
        let input = Tensor::new(&[count, 3], &values, &memory)?;
        let started = Instant::now();
        let output = stream.execute(graph.identity(), stream.next_sequence(), &input)?;
        execution_seconds += started.elapsed().as_secs_f64();
        for (row, actual) in output.output.values().as_chunks::<2>().0.iter().enumerate() {
            let arguments = &values[row * 3..row * 3 + 3];
            let a = direct_triangle.call("sample", arguments)?;
            let b = direct_bell.call("sample", arguments)?;
            let dry = a * (1. - 0.35) + b * 0.35;
            let scaled = dry * 1.5;
            let wet = scaled / (1. + scaled.abs());
            if actual[0].to_bits() != dry.to_bits() || actual[1].to_bits() != wet.to_bits() {
                return Err(
                    "FluxR effect graph differs from independent Rust effect schedule".into(),
                );
            }
            add_values(&mut dry_digest, &actual[..1]);
            add_values(&mut wet_digest, &actual[1..]);
            peak = peak.max(actual[1].abs());
            sum_sq += f64::from(actual[1]) * f64::from(actual[1]);
        }
    }
    let dry_hash = format!("{:x}", dry_digest.finalize());
    let wet_hash = format!("{:x}", wet_digest.finalize());
    if dry_hash == wet_hash {
        return Err("new external effect produced unchanged audio fixture".into());
    }
    Ok(
        json!({"schema":"fluxr-native-stream-effects-v1","graph_identity":graph.identity(),"graph_spec":serde_json::from_slice::<Value>(&graph.encode())?,"backend":"FluxR portable CPU evaluator for instruments, mixing and soft clipping","library_identities":[triangle.identity(),bell.identity(),effect.identity()],"source_sha256":[triangle.manifest().source_sha256,bell.manifest().source_sha256,effect.manifest().source_sha256],"preparation_seconds":preparation_seconds,"execution_seconds":execution_seconds,"sample_rate":8000,"frames":rows,"duration_seconds":1,"chunk_rows":chunk_rows,"dry_sample_bits_sha256":dry_hash,"wet_sample_bits_sha256":wet_hash,"peak_abs":peak,"rms":(sum_sq/rows as f64).sqrt(),"payload_peak_bytes":memory.peak_bytes(),"stream_contract":stream.contract(),"stream_contract_identity":stream.contract_identity(),"statistics":stream.statistics(),"attempts":stream.attempts(),"real_time_qualified":false,"limitations":["One-second instrument/effect fixture complements the inherited 16-second multitrack composition","This is an offline scalar graph; no audio device callback, graph swap, latency negotiation or foreign plugin ABI is executed"]}),
    )
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(
            "usage: native_stream_demo (ml|effects) NATIVE_FIXTURE_ROOT OUTPUT_JSON".into(),
        );
    }
    let report = match args[0].as_str() {
        "ml" => ml(Path::new(&args[1]))?,
        "effects" => effects(Path::new(&args[1]))?,
        _ => return Err("unknown native stream mode".into()),
    };
    fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::to_string(&json!({"schema":report["schema"],"output":args[2],"passed":true}))?
    );
    Ok(())
}
