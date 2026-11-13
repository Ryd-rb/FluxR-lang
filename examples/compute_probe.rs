//! Reproducible compile/reflect/CPU/Vulkan probe; heavyweight backend is opt-in.
use fluxr::{compute::Kernel, structured::Module};
use std::{fs, io::Read, time::Instant};
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
#[path = "fixtures/vkf1_reference.rs"]
mod reference;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [mode, source, output] = args.as_slice() else {
        return Err("usage: compute_probe <emit|gpu> source output".into());
    };
    let mut text = String::new();
    fs::File::open(source)?
        .take(256 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 256 * 1024 {
        return Err("source budget exceeded".into());
    }
    let start = Instant::now();
    let module = if text.starts_with("fa1") {
        Module::compile_agent(&text)?
    } else {
        Module::compile_compact(&text)?
    };
    let kernel = Kernel::compile(&module, "kernel", 64)?;
    let compile_ns = start.elapsed().as_nanos();
    match mode.as_str() {
        "emit" => {
            fs::write(output, kernel.bytes())?;
            println!("{}", serde_json::to_string_pretty(kernel.reflection())?);
        }
        "gpu" => gpu(&kernel, compile_ns, output)?,
        _ => return Err("unknown probe mode".into()),
    }
    Ok(())
}

#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
fn tensor_bridge(
    executor: &mut fluxr::vulkan::Executor,
    kernel: &Kernel,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use fluxr::cpu_compute::{CpuBatchExecutor, MemoryBudget, ScalarLibrary, Tensor};
    use fluxr::structured::Limits;
    use fluxr::vulkan::DispatchLimits;
    use std::sync::{Arc, atomic::AtomicBool};
    let cancel = AtomicBool::new(false);
    let memory = MemoryBudget::new(8 << 20);
    let a = Tensor::new(&[2, 2], &[1.0, 2.0, 3.0, 4.0], &memory)?;
    let b = Tensor::new(&[2, 2], &[0.5, 0.5, 0.5, 0.5], &memory)?;
    let before = memory.live_bytes();
    let result = executor.run_tensors(
        kernel,
        &[a.clone(), b.clone()],
        &memory,
        DispatchLimits::default(),
        &cancel,
    )?;
    assert_eq!(result.output.shape(), &[2, 2]);
    assert_eq!(result.output.values(), &[2.5, 4.5, 6.5, 8.5]);
    assert_eq!(result.numerical_profile, fluxr::compute::PROFILE);
    assert_eq!(memory.live_bytes(), before + 16);
    drop(result);
    assert_eq!(memory.live_bytes(), before);
    let other_shape = Tensor::new(&[4], b.values(), &memory)?;
    let zero = MemoryBudget::new(0);
    let shape_refusal = executor
        .run_tensors(
            kernel,
            &[a.clone(), other_shape],
            &memory,
            DispatchLimits::default(),
            &cancel,
        )
        .err()
        .ok_or("shape should refuse")?;
    let quota_refusal = executor
        .run_tensors(
            kernel,
            &[a.clone(), b.clone()],
            &zero,
            DispatchLimits::default(),
            &cancel,
        )
        .err()
        .ok_or("quota should refuse")?;
    assert_eq!(zero.live_bytes(), 0);
    let overflow = Tensor::new(&[2, 2], &[f32::MAX; 4], &memory)?;
    let before = memory.live_bytes();
    let overflow_refusal = executor
        .run_tensors(
            kernel,
            &[overflow.clone(), b.clone()],
            &memory,
            DispatchLimits::default(),
            &cancel,
        )
        .err()
        .ok_or("nonfinite output should refuse")?;
    assert_eq!(memory.live_bytes(), before);
    drop((a, b, overflow));
    assert_eq!(memory.live_bytes(), 0);

    // The same checked FluxR module supplies CPU neural and Vulkan affine
    // exports. Both are admitted before collecting either result. This observes
    // placement/concurrent admission, not simultaneous physical GPU execution.
    let source = include_str!("native/ml/program.flux");
    let library = Arc::new(ScalarLibrary::load(
        include_bytes!("native/ml/manifest.json"),
        source,
    )?);
    let model = Module::compile_compact(source)?;
    let gpu_model = Kernel::compile(&model, "affine", 64)?;
    let rows = 65536;
    let x: Vec<f32> = (0..rows).map(|i| i as f32 / 32.0).collect();
    let weight = vec![2.0; rows];
    let bias = vec![1.0; rows];
    let gpu_inputs = [
        Tensor::new(&[rows], &x, &memory)?,
        Tensor::new(&[rows], &weight, &memory)?,
        Tensor::new(&[rows], &bias, &memory)?,
    ];
    let cpu_rows: Vec<f32> = x.iter().flat_map(|&x| [x, 1.0, 0.0, 2.0, 1.0]).collect();
    let cpu_input = Tensor::new(&[rows, 5], &cpu_rows, &memory)?;
    let cpu = CpuBatchExecutor::new(1)?;
    let job = cpu.submit(
        library.clone(),
        "predict".into(),
        cpu_input,
        memory.clone(),
        Limits::default(),
        rows as u64,
    )?;
    let cpu_pending_before = !job.is_finished();
    let gpu_result = executor.run_tensors(
        &gpu_model,
        &gpu_inputs,
        &memory,
        DispatchLimits::default(),
        &cancel,
    )?;
    let cpu_pending_after = !job.is_finished();
    let cpu_output = job.wait()?;
    assert!(
        cpu_output
            .values()
            .iter()
            .zip(gpu_result.output.values())
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    let record = serde_json::json!({"shared_tensor_abi":"FTB1 unchanged","same_shape_checked":true,"output_charge_released":true,"refusals":[shape_refusal.to_string(),quota_refusal.to_string(),overflow_refusal.to_string()],"operation_numeric_profile":gpu_result.numerical_profile,"gpu_output_storage":"owned CPU tensor after explicit transfer","cpu_model":"predict","vulkan_model":"affine","source_module_sha256":gpu_result.source_sha256,"cpu_library_identity":library.identity(),"rows_per_model":rows,"cpu_pending_before_vulkan":cpu_pending_before,"cpu_pending_after_vulkan":cpu_pending_after,"results_bitwise_equal":true,"payload_peak_bytes":memory.peak_bytes(),"device":gpu_result.device,"vulkan_timings":gpu_result.timings,"concurrency_scope":"both jobs admitted before collecting CPU result; scheduling/overlap observations only; software Vulkan remains CPU hardware"});
    drop((gpu_result, cpu_output, gpu_inputs));
    assert_eq!(memory.live_bytes(), 0);
    Ok(record)
}
#[cfg(not(all(feature = "vulkan", not(target_arch = "wasm32"))))]
fn gpu(_: &Kernel, _: u128, _: &str) -> Result<(), Box<dyn std::error::Error>> {
    Err("gpu mode requires --features vulkan".into())
}

#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
fn gpu(kernel: &Kernel, compile_ns: u128, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    use fluxr::vulkan::{DispatchLimits, Executor};
    use std::sync::atomic::AtomicBool;
    let init = Instant::now();
    let mut executor = Executor::new()?;
    let context_ns = init.elapsed().as_nanos();
    let cancelled = AtomicBool::new(false);
    let mut cases = vec![];
    for count in [1_usize, 63, 64, 65, 1025, 65537] {
        let x: Vec<f32> = (0..count)
            .map(|i| (i as i32 % 4096 - 2048) as f32 / 16.0)
            .collect();
        let y: Vec<f32> = (0..count)
            .map(|i| (i as i32 % 256 - 128) as f32 / 32.0)
            .collect();
        let cpu = Instant::now();
        let reference = kernel.cpu_reference(&[&x, &y], 1 << 20)?;
        let cpu_ns = cpu.elapsed().as_nanos();
        let mut rust_samples_ns = vec![];
        for _ in 0..5 {
            let rust_start = Instant::now();
            let values: Vec<_> = std::hint::black_box(&x)
                .iter()
                .zip(std::hint::black_box(&y))
                .map(|(&x, &y)| reference::kernel(x, y))
                .collect();
            rust_samples_ns.push(rust_start.elapsed().as_nanos());
            if values
                .iter()
                .zip(&reference)
                .any(|(a, b)| a.to_bits() != b.to_bits())
            {
                return Err("matched Rust differs from FluxR VM".into());
            }
            std::hint::black_box(values);
        }
        let mut samples = vec![];
        for _ in 0..5 {
            let (values, timings) =
                executor.run(kernel, &[&x, &y], DispatchLimits::default(), &cancelled)?;
            let mismatches = values
                .iter()
                .zip(&reference)
                .filter(|(a, b)| a.to_bits() != b.to_bits())
                .count();
            if mismatches != 0 {
                return Err(format!("GPU differs from CPU in {mismatches} lanes").into());
            }
            samples.push(serde_json::to_value(timings)?);
        }
        cases.push(serde_json::json!({"elements":count,"cpu_vm_ns":cpu_ns,"rust_scalar_map_with_output_allocation_ns":rust_samples_ns,"bitwise_mismatches":0,"samples":samples}));
    }
    let x = [1.0, 2.0];
    let wrong = [1.0];
    let finite = [f32::NAN, 1.0];
    let mut refusals = vec![];
    for (name, inputs, limits, cancel) in [
        (
            "unequal_shape",
            vec![x.as_slice(), wrong.as_slice()],
            DispatchLimits::default(),
            false,
        ),
        (
            "nan_input",
            vec![x.as_slice(), finite.as_slice()],
            DispatchLimits::default(),
            false,
        ),
        (
            "memory_budget",
            vec![x.as_slice(), x.as_slice()],
            DispatchLimits {
                max_allocation_bytes: 1,
                ..Default::default()
            },
            false,
        ),
        (
            "element_budget",
            vec![x.as_slice(), x.as_slice()],
            DispatchLimits {
                max_elements: 1,
                ..Default::default()
            },
            false,
        ),
        (
            "pre_cancelled",
            vec![x.as_slice(), x.as_slice()],
            DispatchLimits::default(),
            true,
        ),
    ] {
        let error = executor
            .run(kernel, &inputs, limits, &AtomicBool::new(cancel))
            .err()
            .ok_or("expected refusal")?;
        refusals.push(serde_json::json!({"case":name,"error":error.to_string()}));
    }
    let tensor_bridge = tensor_bridge(&mut executor, kernel)?;
    let record = serde_json::json!({"schema":"fluxr-vkf1-execution-v1","device":executor.info(),"kernel":kernel.reflection(),"compile_ns":compile_ns,"context_creation_ns":context_ns,"cases":cases,"refusals":refusals,"tensor_bridge":tensor_bridge,"scope":"actual Vulkan compute; finite dyadic fixture; GPU device type recorded; no physical GPU, all-call execution, strict IEEE or general speedup claim"});
    fs::write(output, serde_json::to_vec_pretty(&record)?)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}
