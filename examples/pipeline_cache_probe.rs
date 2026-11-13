//! Execute the same FluxR kernel with and without opt-in live pipeline reuse.
//! This is a trusted fixed-fixture probe, not arbitrary-candidate containment.
#[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use fluxr::compute::Kernel;
    use fluxr::structured::Module;
    use fluxr::vulkan::{DispatchLimits, Executor, PipelineCacheLimits};
    use std::{sync::atomic::AtomicBool, time::Instant};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [output] = args.as_slice() else {
        return Err("usage: pipeline_cache_probe output.json".into());
    };
    let started = Instant::now();
    let source = include_str!("vkf1-affine.fluxr");
    let module = Module::compile_compact(source)?;
    let kernel = Kernel::compile(&module, "kernel", 64)?;
    let compile_ns = started.elapsed().as_nanos();
    let cancel = AtomicBool::new(false);
    let mut uncached = Executor::new()?;
    let mut cached = Executor::with_pipeline_cache(PipelineCacheLimits {
        max_entries: 2,
        max_key_bytes: 1 << 20,
    })?;
    let mut cases = vec![];
    for count in [1_usize, 65, 4097, 65537] {
        let x: Vec<f32> = (0..count)
            .map(|i| (i as i32 % 512 - 256) as f32 / 16.0)
            .collect();
        let y: Vec<f32> = (0..count)
            .map(|i| (i as i32 % 128 - 64) as f32 / 32.0)
            .collect();
        let reference = kernel.cpu_reference(&[&x, &y], 1 << 20)?;
        let mut trials = vec![];
        for trial in 0..9 {
            // Alternate order to expose order/warmup bias; retain every sample.
            for mode in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let executor = if mode { &mut cached } else { &mut uncached };
                let start = Instant::now();
                let (actual, timings) =
                    executor.run(&kernel, &[&x, &y], DispatchLimits::default(), &cancel)?;
                let end_to_end_ns = start.elapsed().as_nanos();
                assert_eq!(actual.len(), reference.len());
                assert!(
                    actual
                        .iter()
                        .zip(&reference)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                );
                trials.push(serde_json::json!({"trial":trial,"cache_enabled":mode,"end_to_end_ns":end_to_end_ns,"timings":timings,"mismatches":0}));
            }
        }
        cases.push(serde_json::json!({"elements":count,"trials":trials}));
    }
    let measured_stats = cached.pipeline_cache_stats();
    assert_eq!(measured_stats.pipeline_creations, 1);
    assert_eq!(measured_stats.hits, 35);
    assert_eq!(measured_stats.resident_entries, 1);
    assert_eq!(uncached.pipeline_cache_stats().pipeline_creations, 36);
    let mut checks = vec![];
    // Same source, distinct workgroup specialization is a distinct pipeline.
    let k32 = Kernel::compile(&module, "kernel", 32)?;
    let k16 = Kernel::compile(&module, "kernel", 16)?;
    for (k, hit) in [(&k32, false), (&kernel, true), (&k16, false), (&k32, false)] {
        let (actual, time) =
            cached.run(k, &[&[2.0], &[0.5]], DispatchLimits::default(), &cancel)?;
        assert_eq!(actual, [4.5]);
        assert_eq!(time.pipeline_cache_hit, hit);
    }
    assert_eq!(cached.pipeline_cache_stats().resident_entries, 2);
    assert_eq!(cached.pipeline_cache_stats().evictions, 2);
    checks.push("workgroup identity and deterministic LRU eviction");
    let stats_before_refusal = cached.pipeline_cache_stats();
    assert!(
        cached
            .run(
                &kernel,
                &[&[1.0], &[f32::NAN]],
                DispatchLimits::default(),
                &cancel
            )
            .is_err()
    );
    assert!(
        cached
            .run(
                &kernel,
                &[&[1.0], &[1.0]],
                DispatchLimits::default(),
                &AtomicBool::new(true)
            )
            .is_err()
    );
    assert!(
        cached
            .run(
                &kernel,
                &[&[1.0], &[1.0]],
                DispatchLimits {
                    max_allocation_bytes: 0,
                    ..DispatchLimits::default()
                },
                &cancel
            )
            .is_err()
    );
    assert_eq!(cached.pipeline_cache_stats(), stats_before_refusal);
    checks.push("invalid input, cancellation and allocation admission leave cache unchanged");
    cached.clear_pipeline_cache()?;
    assert_eq!(cached.pipeline_cache_stats().resident_entries, 0);
    assert_eq!(cached.pipeline_cache_stats().resident_key_bytes, 0);
    let (_, rebuilt) = cached.run(
        &kernel,
        &[&[1.0], &[0.0]],
        DispatchLimits::default(),
        &cancel,
    )?;
    assert!(!rebuilt.pipeline_cache_hit);
    checks.push("idle clear releases retained identities and next request rebuilds");
    let mut tiny = Executor::with_pipeline_cache(PipelineCacheLimits {
        max_entries: 1,
        max_key_bytes: 1,
    })?;
    for _ in 0..2 {
        let (v, t) = tiny.run(
            &kernel,
            &[&[1.0], &[0.0]],
            DispatchLimits::default(),
            &cancel,
        )?;
        assert_eq!(v, [2.0]);
        assert!(!t.pipeline_cache_hit);
    }
    assert_eq!(tiny.pipeline_cache_stats().bypasses, 2);
    assert_eq!(tiny.pipeline_cache_stats().resident_entries, 0);
    checks.push("oversize exact keys bypass retention without changing execution");
    let report = serde_json::json!({
        "schema":"fluxr-vulkan-live-pipeline-reuse-v1","profile":fluxr::compute::PROFILE,
        "source_sha256":kernel.reflection().source_sha256,"spirv_sha256":kernel.reflection().spirv_sha256,
        "compile_ns":compile_ns,"device":cached.info(),"cases":cases,
        "measured_cached_stats":measured_stats,"measured_uncached_stats":uncached.pipeline_cache_stats(),
        "final_cached_stats":cached.pipeline_cache_stats(),"tiny_cache_stats":tiny.pipeline_cache_stats(),
        "checks":checks,"executed_dispatches":79,
        "limits":"retained live object count and key payload bytes bounded; opaque driver memory not measured or charged; one transient pipeline can coexist during replacement",
        "scope":"same process/runtime, finite dyadic affine fixture; real Vulkan on recorded device; no physical-GPU or cross-device speed guarantee; default constructor stays uncached"
    });
    std::fs::write(output, serde_json::to_string_pretty(&report)? + "\n")?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
#[cfg(not(all(feature = "vulkan", not(target_arch = "wasm32"))))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("pipeline_cache_probe requires --features vulkan".into())
}
