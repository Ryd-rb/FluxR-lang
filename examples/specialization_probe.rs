//! Fixed reviewed FluxR VKS1 program, real Vulkan specialization and matched
//! dynamic-buffer/portable-VM/Rust paths. This is not hostile candidate admission.
use fluxr::{
    compute::{Kernel, Specialization},
    structured::Module,
};

fn module() -> Result<Module, fluxr::Error> {
    Module::compile_agent(include_str!("vks1-affine.fluxr"))
}
fn kernel(scale: f32, bias: f32) -> Result<Kernel, fluxr::Error> {
    Kernel::compile_specialized(
        &module()?,
        "kernel",
        64,
        &[
            Specialization {
                parameter_index: 1,
                value_bits: scale.to_bits(),
            },
            Specialization {
                parameter_index: 2,
                value_bits: bias.to_bits(),
            },
        ],
    )
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [mode, output] = args.as_slice() else {
        return Err("usage: specialization_probe <emit|gpu> output".into());
    };
    match mode.as_str() {
        "emit" => {
            let a = kernel(2.0, 1.0)?;
            let b = kernel(-0.5, 0.25)?;
            assert_eq!(a.bytes(), b.bytes());
            assert_ne!(a.reflection().variant_sha256, b.reflection().variant_sha256);
            std::fs::write(output, a.bytes())?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"a":a.reflection(),"b":b.reflection()})
                )?
            );
        }
        "gpu" => gpu(output)?,
        _ => return Err("unknown probe mode".into()),
    }
    Ok(())
}

#[cfg(not(all(feature = "vulkan", not(target_arch = "wasm32"))))]
fn gpu(_: &str) -> Result<(), Box<dyn std::error::Error>> {
    Err("requires --features vulkan".into())
}

#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
fn gpu(output: &str) -> Result<(), Box<dyn std::error::Error>> {
    use fluxr::{
        cpu_compute::{MemoryBudget, Tensor},
        vulkan::{DispatchLimits, Executor, PipelineCacheLimits},
    };
    use std::{sync::atomic::AtomicBool, time::Instant};
    let init = Instant::now();
    let mut executor = Executor::with_pipeline_cache(PipelineCacheLimits {
        max_entries: 8,
        max_key_bytes: 1 << 20,
    })?;
    let initialization_ns = init.elapsed().as_nanos();
    let cancel = AtomicBool::new(false);
    let compile = Instant::now();
    let dynamic = Kernel::compile(&module()?, "kernel", 64)?;
    let settings = [(2.0, 1.0), (-0.5, 0.25), (4.0, -2.0)];
    let specialized: Vec<_> = settings
        .iter()
        .map(|&(a, b)| kernel(a, b))
        .collect::<Result<_, _>>()?;
    let preparation_ns = compile.elapsed().as_nanos();
    let mut records = vec![];
    for count in [1, 63, 64, 65, 4097, 65537] {
        let x: Vec<_> = (0..count)
            .map(|i| (i as i32 % 4096 - 2048) as f32 / 16.0)
            .collect();
        for (variant, (&(scale, bias), specialized)) in
            settings.iter().zip(&specialized).enumerate()
        {
            let vm_start = Instant::now();
            let expected = specialized.cpu_reference(&[&x], count)?;
            let vm_ns = vm_start.elapsed().as_nanos();
            let columns_start = Instant::now();
            let scales = vec![scale; count];
            let biases = vec![bias; count];
            let dynamic_column_setup_ns = columns_start.elapsed().as_nanos();
            let mut trials = vec![];
            for trial in 0..5 {
                let start = Instant::now();
                let rust: Vec<_> = std::hint::black_box(&x)
                    .iter()
                    .map(|&v| v * std::hint::black_box(scale) + std::hint::black_box(bias))
                    .collect();
                let rust_ns = start.elapsed().as_nanos();
                assert_eq!(rust.len(), expected.len());
                assert!(
                    rust.iter()
                        .zip(&expected)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                );
                std::hint::black_box(rust);
                let mut timings = serde_json::Map::new();
                let order = if trial % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                };
                for is_specialized in order {
                    let (got, timing) = if is_specialized {
                        executor.run(specialized, &[&x], DispatchLimits::default(), &cancel)?
                    } else {
                        executor.run(
                            &dynamic,
                            &[&x, &scales, &biases],
                            DispatchLimits::default(),
                            &cancel,
                        )?
                    };
                    if got.len() != expected.len()
                        || !got
                            .iter()
                            .zip(&expected)
                            .all(|(a, b)| a.to_bits() == b.to_bits())
                    {
                        return Err(format!("variant {variant} count {count} trial {trial} specialization {is_specialized} differs").into());
                    }
                    timings.insert(
                        if is_specialized {
                            "specialized"
                        } else {
                            "dynamic_buffers"
                        }
                        .into(),
                        serde_json::to_value(timing)?,
                    );
                }
                trials.push(serde_json::json!({"trial":trial,"specialized_first":order[0],"rust_output_allocation_ns":rust_ns,"timings":timings}));
            }
            let bytes: Vec<_> = expected
                .iter()
                .flat_map(|v| v.to_bits().to_le_bytes())
                .collect();
            records.push(serde_json::json!({"variant":variant,"elements":count,"reflection":specialized.reflection(),"vm_ns":vm_ns,
                "dynamic_column_setup_ns":dynamic_column_setup_ns,"dynamic_input_payload_bytes":count*12,"specialized_input_payload_bytes":count*4,
                "specialization_value_bytes":8,"output_sha256":fluxr::compute::sha256(&bytes),"bitwise_mismatches":0,"trials":trials}));
        }
    }
    let memory = MemoryBudget::new(1024);
    let x = Tensor::new(&[2, 2], &[1.0, 2.0, 3.0, 4.0], &memory)?;
    let before = memory.live_bytes();
    let result = executor.run_tensors(
        &specialized[0],
        std::slice::from_ref(&x),
        &memory,
        DispatchLimits::default(),
        &cancel,
    )?;
    assert_eq!(result.output.values(), [3.0, 5.0, 7.0, 9.0]);
    assert_eq!(
        result.numerical_profile,
        fluxr::compute::SPECIALIZED_PROFILE
    );
    assert_eq!(
        result.variant_sha256,
        specialized[0].reflection().variant_sha256
    );
    assert_eq!(memory.live_bytes(), before + 16);
    let provenance = serde_json::json!({"profile":result.numerical_profile,"variant_sha256":result.variant_sha256,"storage":"CPU tensor after real Vulkan completion"});
    drop(result);
    assert_eq!(memory.live_bytes(), before);
    let mut refusals = vec![];
    for (name, inputs, limits, cancelled) in [
        (
            "extra_dynamic_binding",
            vec![&[1.0][..], &[2.0][..]],
            DispatchLimits::default(),
            false,
        ),
        (
            "empty_dynamic",
            vec![&[][..]],
            DispatchLimits::default(),
            false,
        ),
        (
            "nonfinite_dynamic",
            vec![&[f32::NAN][..]],
            DispatchLimits::default(),
            false,
        ),
        (
            "allocation_quota",
            vec![&[1.0][..]],
            DispatchLimits {
                max_allocation_bytes: 1,
                ..DispatchLimits::default()
            },
            false,
        ),
        (
            "cancelled",
            vec![&[1.0][..]],
            DispatchLimits::default(),
            true,
        ),
    ] {
        let err = executor
            .run(
                &specialized[0],
                &inputs,
                limits,
                &AtomicBool::new(cancelled),
            )
            .err()
            .ok_or("expected refusal")?;
        assert!(!executor.has_pending());
        refusals.push(serde_json::json!({"case":name,"error":err.to_string()}));
    }
    let report = serde_json::json!({"schema":"fluxr-vks1-qualification-v1","device":executor.info(),"initialization_ns":initialization_ns,"kernel_preparation_ns":preparation_ns,
        "cases":records,"cache":executor.pipeline_cache_stats(),"tensor_provenance":provenance,"refusals":refusals,
        "numeric_scope":"finite dyadic fixture bitwise comparisons only; explicitly approximate GPU profile; no general strict equivalence",
        "memory_scope":"actual Vulkan padded allocations per trial plus host payload; driver opaque memory excluded",
        "measurement_scope":"alternating same-process real Vulkan vs dynamic-buffer Vulkan, independent portable VM and matched Rust with output allocation; software device is CPU hardware"});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!("VKS1: 180 paired-matrix dispatches, one tensor dispatch, five admission refusals");
    Ok(())
}
