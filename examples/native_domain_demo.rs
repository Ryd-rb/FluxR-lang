//! Reproducible native-domain qualification driver. All algorithms/instruments
//! below are loaded from external FluxR packages; this binary does not embed
//! their source. It never executes arbitrary native code from a package.
use fluxr::cpu_compute::{
    Cancellation, CpuBatchExecutor, MemoryBudget, ModelCheckpoint, ScalarLibrary, SparseCsr, Tensor,
};
use fluxr::offline_audio::{Composition, RenderLimits, render};
use fluxr::structured::Limits;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

#[path = "native/reference.rs"]
mod reference;

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(format!("file exceeds {limit}-byte limit: {}", path.display()).into());
    }
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("file grew beyond byte limit".into());
    }
    Ok(bytes)
}
fn load(path: &Path) -> Result<ScalarLibrary, Box<dyn Error>> {
    let manifest = read_bounded(&path.join("manifest.json"), 16384)?;
    let source = String::from_utf8(read_bounded(&path.join("program.flux"), 256 * 1024)?)?;
    Ok(ScalarLibrary::load(&manifest, &source)?)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn ml(root: &Path) -> Result<Value, Box<dyn Error>> {
    let prepare = Instant::now();
    let library = load(root)?;
    let preparation_seconds = prepare.elapsed().as_secs_f64();
    let cancellation = Cancellation::default();
    let memory = MemoryBudget::new(8 * 1024 * 1024);
    let mut session = library.session(Limits::default(), 100000, cancellation.clone());
    let dataset = [(0.25f32, 1.5f32), (0.5, 2.), (0.75, 2.5), (1., 3.)];
    let mut parameters = [0.5f32, 0.2, 0.7, -0.1];
    let mut initial_loss = 0f32;
    for &(x, y) in &dataset {
        initial_loss += session.call(
            "loss",
            &[
                x,
                y,
                parameters[0],
                parameters[1],
                parameters[2],
                parameters[3],
            ],
        )?;
    }
    let start = Instant::now();
    let epochs = 600;
    for _ in 0..epochs {
        for &(x, y) in &dataset {
            let before = parameters;
            for (direction, parameter) in parameters.iter_mut().enumerate() {
                *parameter = session.call(
                    "step",
                    &[
                        x,
                        y,
                        before[0],
                        before[1],
                        before[2],
                        before[3],
                        0.03,
                        direction as f32,
                    ],
                )?;
            }
        }
    }
    let training_seconds = start.elapsed().as_secs_f64();
    let reference_start = Instant::now();
    let reference_parameters = std::hint::black_box(reference::train(
        std::hint::black_box(&dataset),
        epochs as usize,
        std::hint::black_box([0.5, 0.2, 0.7, -0.1]),
    ));
    let reference_training_seconds = reference_start.elapsed().as_secs_f64();
    if parameters.map(f32::to_bits) != reference_parameters.map(f32::to_bits) {
        return Err("training strict Rust schedule differs at parameter bits".into());
    }
    let mut final_loss = 0f32;
    let mut predictions = Vec::new();
    for &(x, y) in &dataset {
        final_loss += session.call(
            "loss",
            &[
                x,
                y,
                parameters[0],
                parameters[1],
                parameters[2],
                parameters[3],
            ],
        )?;
        predictions.push(session.call(
            "predict",
            &[
                x,
                parameters[0],
                parameters[1],
                parameters[2],
                parameters[3],
            ],
        )?);
    }
    if final_loss >= 1e-7 || final_loss >= initial_loss {
        return Err(format!(
            "training acceptance failed: initial={initial_loss} final={final_loss}"
        )
        .into());
    }
    let checkpoint = ModelCheckpoint::new(
        &library,
        &parameters,
        epochs * 4,
        0,
        "FluxR forward-mode dual-number SGD; deterministic input order",
    )?;
    let checkpoint_bytes = checkpoint.encode()?;
    let restored = ModelCheckpoint::decode(&checkpoint_bytes, &library, 4)?;
    if checkpoint.parameter_bits != restored.parameter_bits {
        return Err("checkpoint roundtrip bits differ".into());
    }
    let imported_bytes = read_bounded(&root.join("pretrained.json"), 65536)?;
    let imported = ModelCheckpoint::decode(&imported_bytes, &library, 4)?;
    let p = imported.parameters();
    let imported_prediction = session.call("predict", &[0.625, p[0], p[1], p[2], p[3]])?;
    if imported_prediction.to_bits() != 2.25f32.to_bits() {
        return Err("pretrained prediction differs from analytic reference".into());
    }
    // Matched scalar functions cover distinct non-neural algorithm families.
    let non_neural = [
        ("linear", session.call("affine", &[3., 2., 1.])?, 7f32),
        (
            "tree_stump",
            session.call("stump", &[0.8, 0.5, -1., 1.])?,
            1.,
        ),
        (
            "nearest_centroid",
            session.call("cluster", &[3.5, 0., 4.])?,
            1.,
        ),
        (
            "rational_kernel",
            session.call("kernel", &[2., 0., 0.25])?,
            0.5,
        ),
    ];
    for &(name, actual, expected) in &non_neural {
        if actual.to_bits() != expected.to_bits() {
            return Err(format!("{name} CPU reference mismatch").into());
        }
    }
    let rows = 4096usize;
    let mut data = Vec::with_capacity(rows * 3);
    for index in 0..rows {
        data.extend_from_slice(&[index as f32 * 0.001, 2., 1.]);
    }
    let input = Tensor::new(&[rows, 3], &data, &memory)?;
    let start = Instant::now();
    let output = session.batch("affine", &input, &memory)?;
    let batch_seconds = start.elapsed().as_secs_f64();
    for (index, &value) in output.values().iter().enumerate() {
        let expected = (index as f32 * 0.001) * 2. + 1.;
        if value.to_bits() != expected.to_bits() {
            return Err("dense batch Rust reference differs".into());
        }
    }
    let sparse = SparseCsr::new(2, 3, &[0, 2, 3], &[0, 2, 1], &[1., 2., 3.], &memory)?;
    let vector = Tensor::new(&[3], &[2., 3., 4.], &memory)?;
    let sparse_result = sparse.matvec(&vector, &memory, &cancellation)?;
    if sparse_result.values() != [10., 9.] {
        return Err("CSR reference differs".into());
    }
    // Each concurrent lane owns an independent mutable VM workspace. One shared
    // immutable program and payload budget do not imply GPU placement.
    let concurrent_start = Instant::now();
    let executor = CpuBatchExecutor::new(4)?;
    let shared_library = std::sync::Arc::new(library.clone());
    let mut jobs = Vec::new();
    for lane in 0..4 {
        let lane_input = Tensor::new(&[rows, 3], &data_for_lane(rows, lane), &memory)?;
        jobs.push((
            lane,
            executor.submit(
                shared_library.clone(),
                "affine".into(),
                lane_input,
                memory.clone(),
                Limits::default(),
                rows as u64,
            )?,
        ));
    }
    let mut concurrent = Vec::new();
    for (lane, job) in jobs {
        let out = job.wait()?;
        for (index, &actual) in out.values().iter().enumerate() {
            let expected = (index as f32 * 0.001) * (lane + 1) as f32 + 1.;
            if actual.to_bits() != expected.to_bits() {
                return Err("concurrent lane reference mismatch".into());
            }
        }
        concurrent.push((lane, rows as u64, hash(&out.encode())));
    }
    let concurrent_seconds = concurrent_start.elapsed().as_secs_f64();
    Ok(
        json!({"schema":"fluxr-ml-demo-v1","program_identity":library.identity(),"backend":"FluxR portable CPU evaluator","preparation_seconds":preparation_seconds,
        "training":{"kind":"one-hidden-ReLU custom neural model","derivative":"forward dual-number arithmetic authored in FluxR","relu_zero_derivative":0,"epochs":epochs,"completed_steps":epochs*4,"initial_half_squared_error_sum":initial_loss,"final_half_squared_error_sum":final_loss,"parameters":parameters,"predictions":predictions,"seconds":training_seconds,"rust_reference_seconds":reference_training_seconds,"fluxr_over_rust_ratio":training_seconds/reference_training_seconds,"rust_parameter_bits_exact":true,"rng":"none; deterministic initial parameters; seed recorded as 0"},
        "checkpoint":{"sha256":hash(&checkpoint_bytes),"roundtrip_exact_bits":true,"imported_fixture_sha256":hash(&imported_bytes),"imported_prediction":imported_prediction,"route":"versioned FluxR checkpoint with analytical pretrained parameters; not ONNX"},
        "non_neural":non_neural.iter().map(|(family,actual,expected)|json!({"family":family,"actual":actual,"reference":expected,"bit_exact":true})).collect::<Vec<_>>(),
        "dense_batch":{"rows":rows,"seconds":batch_seconds,"rows_per_second":rows as f64/batch_seconds,"bit_exact_rust_reference":true,"transfer":"host memory copy at Tensor construction; no device transfer"},
        "sparse":{"shape":[2,3],"format":"canonical CSR","result":sparse_result.values()},
        "concurrent_cpu":{"lanes":4,"peak_in_flight":executor.peak_active_jobs(),"scheduler":"bounded CpuBatchExecutor; explicit completion","rows_per_lane":rows,"seconds":concurrent_seconds,"results":concurrent,"gpu_executed":false},
        "payload_peak_bytes":memory.peak_bytes(),"attempted_calls":session.attempted_calls(),"completed_calls":session.completed_calls(),"all_attempts_retained":true,
        "limitations":["No ONNX or vendor model zoo import", "No GPU placement or reverse-mode AD", "Payload peak excludes VM, host data and process RSS", "Analytical tiny dataset is a correctness fixture, not a general ML benchmark"]}),
    )
}
fn data_for_lane(rows: usize, lane: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(rows * 3);
    for index in 0..rows {
        data.extend_from_slice(&[index as f32 * 0.001, (lane + 1) as f32, 1.]);
    }
    data
}
fn wav(path: &Path, sample_rate: u32, samples: &[f32]) -> Result<(), Box<dyn Error>> {
    let bytes = u32::try_from(samples.len().checked_mul(2).ok_or("WAV length overflow")?)?;
    let mut file = fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(bytes + 36).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 4).to_le_bytes())?;
    file.write_all(&4u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&bytes.to_le_bytes())?;
    for &sample in samples {
        let pcm = (sample.clamp(-1., 1.) * 32767.).round() as i16;
        file.write_all(&pcm.to_le_bytes())?;
    }
    Ok(())
}
fn audio(root: &Path, wav_path: Option<&Path>) -> Result<Value, Box<dyn Error>> {
    let prepare = Instant::now();
    let libraries = ["triangle", "pluck", "bell", "noise"]
        .map(|name| load(&root.join("instruments").join(name)))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let composition_bytes = read_bounded(&root.join("composition.json"), 256 * 1024)?;
    let composition: Composition = serde_json::from_slice(&composition_bytes)?;
    let preparation_seconds = prepare.elapsed().as_secs_f64();
    let memory = MemoryBudget::new(16 * 1024 * 1024);
    let cancel = Cancellation::default();
    let rendered = render(
        &composition,
        &libraries,
        RenderLimits::default(),
        &memory,
        &cancel,
    )?;
    let bytes = rendered.samples.encode();
    if let Some(path) = wav_path {
        wav(path, composition.sample_rate, rendered.samples.values())?;
    }
    let clipped = rendered
        .samples
        .values()
        .iter()
        .filter(|v| v.abs() > 1.)
        .count();
    if clipped != 0 {
        return Err("composition exceeded the no-clipping acceptance gate".into());
    }
    Ok(
        json!({"schema":"fluxr-audio-demo-v1","composition_sha256":hash(&composition_bytes),"tensor_sha256":hash(&bytes),"preparation_seconds":preparation_seconds,"report":rendered.report,"clipped_samples":clipped,"backends":{"instruments":"external FluxR CPU portable evaluator","mix_filter_delay":"fixed-order checked Rust host adapter","device_output":"not executed"},"limitations":["Offline renderer; no real-time callback qualification","No foreign CLAP/VST ABI or device-latency measurement","Block deadlines are elapsed wall-time observations, not a hard scheduling guarantee","Output audio not included in pack; reproduce from compact score and instrument programs"]}),
    )
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        return Err("usage: native_domain_demo (ml|audio|probe|make-pretrained) PACKAGE_ROOT OUTPUT_JSON [OUTPUT_WAV]".into());
    }
    let mode = &args[0];
    let root = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let result = match mode.as_str() {
        "ml" => ml(&root)?,
        "audio" => audio(&root, args.get(3).map(Path::new))?,
        "probe" => {
            let library = load(&root)?;
            let mut session = library.session(Limits::default(), 1, Cancellation::default());
            let sample = session.call("sample", &[0.23, 0.17, 0.7])?;
            json!({"schema":"fluxr-library-probe-v1","program_identity":library.identity(),"package":library.manifest().package,"result_bits":sample.to_bits(),"calls":session.completed_calls(),"backend":"FluxR portable CPU evaluator"})
        }
        "make-pretrained" => {
            let library = load(&root)?;
            let checkpoint = ModelCheckpoint::new(
                &library,
                &[1., 0., 2., 1.],
                0,
                0,
                "Analytical fixture: ReLU(x)*2+1 for positive x; parameters are not from a learned model zoo",
            )?;
            serde_json::to_value(checkpoint)?
        }
        _ => return Err("unknown native demo mode".into()),
    };
    write_json(&output, &result)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
