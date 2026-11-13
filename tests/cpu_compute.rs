use fluxr::cpu_compute::{
    ABI, Cancellation, LibraryManifest, MemoryBudget, ModelCheckpoint, NUMERIC_PROFILE,
    ScalarLibrary, SparseCsr, Tensor,
};
use fluxr::structured::Limits;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn package(source: &str, exports: &[(&str, usize)]) -> (Vec<u8>, String) {
    let manifest = LibraryManifest {
        abi: ABI.into(),
        numeric_profile: NUMERIC_PROFILE.into(),
        source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        package: "test.library".into(),
        version: 1,
        exports: exports
            .iter()
            .map(|(name, arity)| (name.to_string(), *arity))
            .collect(),
        capabilities: vec!["compute.cpu".into()],
    };
    (serde_json::to_vec(&manifest).unwrap(), source.into())
}
fn library(source: &str, exports: &[(&str, usize)]) -> ScalarLibrary {
    let (m, s) = package(source, exports);
    ScalarLibrary::load(&m, &s).unwrap()
}
#[test]
fn tensor_shape_bits_lifetime_and_memory_are_checked() {
    let memory = MemoryBudget::new(16);
    let tensor = Tensor::new(&[2, 2], &[0., -0., 1., -2.], &memory).unwrap();
    assert_eq!(memory.live_bytes(), 16);
    assert_eq!(tensor.device(), fluxr::cpu_compute::Device::Cpu);
    assert_eq!(tensor.alignment(), 4);
    let alias = tensor.clone();
    drop(tensor);
    assert_eq!(memory.live_bytes(), 16);
    assert!(Tensor::new(&[1], &[1.], &memory).is_err());
    assert_eq!(memory.live_bytes(), 16);
    let encoded = alias.encode();
    drop(alias);
    assert_eq!(memory.live_bytes(), 0);
    let decoded = Tensor::decode(&encoded, &memory).unwrap();
    assert_eq!(
        decoded
            .values()
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        vec![0, 0x80000000, 1f32.to_bits(), (-2f32).to_bits()]
    );
    assert_eq!(decoded.encode(), encoded);
    assert_eq!(memory.peak_bytes(), 16);
}
#[test]
fn malformed_tensor_cannot_hide_overflow_or_nonfinite_bits() {
    let memory = MemoryBudget::new(1024);
    assert!(Tensor::new(&[usize::MAX, 0, 2], &[], &memory).is_err());
    assert!(Tensor::new(&[1; 9], &[1.], &memory).is_err());
    assert!(Tensor::new(&[2], &[1.], &memory).is_err());
    assert!(Tensor::new(&[1], &[f32::NAN], &memory).is_err());
    assert!(Tensor::new(&[1], &[f32::INFINITY], &memory).is_err());
    let encoded = Tensor::new(&[1], &[1.], &memory).unwrap().encode();
    for cut in 0..encoded.len() {
        assert!(Tensor::decode(&encoded[..cut], &memory).is_err());
    }
    let mut extra = encoded.clone();
    extra.push(0);
    assert!(Tensor::decode(&extra, &memory).is_err());
    for byte in [4usize, 5, 6, 7] {
        let mut bad = encoded.clone();
        bad[byte] = 255;
        assert!(Tensor::decode(&bad, &memory).is_err());
    }
    let mut bad = encoded;
    bad[16..20].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert!(Tensor::decode(&bad, &memory).is_err());
    assert_eq!(memory.live_bytes(), 0);
}
#[test]
fn sparse_canonicalization_and_order_match_scalar_reference() {
    let m = MemoryBudget::new(4096);
    let csr = SparseCsr::new(3, 4, &[0, 2, 2, 3], &[0, 3, 2], &[2., -1., 0.5], &m).unwrap();
    let vector = Tensor::new(&[4], &[3., 8., 4., 7.], &m).unwrap();
    let answer = csr.matvec(&vector, &m, &Cancellation::default()).unwrap();
    assert_eq!(answer.values(), &[-1., 0., 2.]);
    assert!(SparseCsr::new(1, 2, &[0, 2], &[0, 0], &[1., 2.], &m).is_err());
    assert!(SparseCsr::new(1, 2, &[0, 2], &[1, 0], &[1., 2.], &m).is_err());
    assert!(SparseCsr::new(1, 2, &[0, 2], &[0, 2], &[1., 2.], &m).is_err());
    assert!(SparseCsr::new(2, 2, &[0, 2, 1], &[0], &[1.], &m).is_err());
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(csr.matvec(&vector, &m, &cancelled).is_err());
}
#[test]
fn package_abi_capability_and_source_pins_are_authoritative_checks() {
    let source = "v2 (pub square (x:f32) f32 = (* x x))";
    let (manifest, source) = package(source, &[("square", 1)]);
    assert!(ScalarLibrary::load(&manifest, &source).is_ok());
    assert!(ScalarLibrary::load(&manifest, &format!("{source} ")).is_err());
    let original: LibraryManifest = serde_json::from_slice(&manifest).unwrap();
    for field in ["abi", "numeric_profile", "capabilities", "exports"] {
        let mut bad = original.clone();
        match field {
            "abi" => bad.abi = "future".into(),
            "numeric_profile" => bad.numeric_profile = "fast-math".into(),
            "capabilities" => bad.capabilities.push("file.read".into()),
            _ => bad.exports = BTreeMap::from([("square".into(), 2)]),
        }
        assert!(ScalarLibrary::load(&serde_json::to_vec(&bad).unwrap(), &source).is_err());
    }
    let (m, s) = package(
        "v2 (inputs ambient:f32) (pub square (x:f32) f32 = (+ x $ambient))",
        &[("square", 1)],
    );
    assert!(ScalarLibrary::load(&m, &s).is_err());
    let (m, s) = package("v2 (pub square (x:i64) i64 = (* x x))", &[("square", 1)]);
    assert!(ScalarLibrary::load(&m, &s).is_err());
}
#[test]
fn library_attempts_traps_cancellation_and_output_atomicity_are_distinct() {
    let lib = library("v2 (pub square (x:f32) f32 = (* x x))", &[("square", 1)]);
    let token = Cancellation::default();
    let mut s = lib.session(Limits::default(), 3, token.clone());
    assert!(s.call("missing", &[1.]).is_err());
    assert!(s.call("square", &[f32::NAN]).is_err());
    assert_eq!(s.call("square", &[3.]).unwrap(), 9.);
    assert!(s.call("square", &[4.]).is_err());
    assert_eq!((s.attempted_calls(), s.completed_calls()), (3, 1));
    let mut s = lib.session(Limits::default(), 3, token.clone());
    token.cancel();
    assert!(s.call("square", &[1.]).is_err());
    assert_eq!(s.attempted_calls(), 0);
    let m = MemoryBudget::new(128);
    let input = Tensor::new(&[2, 1], &[2., f32::MAX], &m).unwrap();
    let mut s = lib.session(Limits::default(), 2, Cancellation::default());
    assert!(s.batch("square", &input, &m).is_err());
    assert_eq!(m.live_bytes(), 8); // no partial output survives a late numerical trap
    assert_eq!((s.attempted_calls(), s.completed_calls()), (2, 1));
    let mut s = lib.session(
        Limits {
            instructions: 0,
            ..Limits::default()
        },
        1,
        Cancellation::default(),
    );
    assert!(
        s.call("square", &[1.])
            .unwrap_err()
            .to_string()
            .contains("instruction budget")
    );
}
#[test]
fn zero_arity_rows_are_not_lost_and_dense_batches_match_scalar_calls() {
    let lib = library(
        "v2 (pub constant () f32 = 7f) (pub affine (x w b:f32) f32 = (+ (* x w) b))",
        &[("constant", 0), ("affine", 3)],
    );
    let m = MemoryBudget::new(128);
    let mut s = lib.session(Limits::default(), 6, Cancellation::default());
    let empty = Tensor::new(&[2, 0], &[], &m).unwrap();
    assert_eq!(s.batch("constant", &empty, &m).unwrap().values(), &[7., 7.]);
    let input = Tensor::new(&[2, 3], &[2., 3., 1., -1., 4., 2.], &m).unwrap();
    assert_eq!(s.batch("affine", &input, &m).unwrap().values(), &[7., -2.]);
    assert_eq!(s.call("affine", &[2., 3., 1.]).unwrap(), 7.);
    assert_eq!(s.call("affine", &[-1., 4., 2.]).unwrap(), -2.);
}
#[test]
fn model_checkpoints_bind_actual_program_identity_and_preserve_bits() {
    let lib = library("v2 (pub identity (x:f32) f32 = x)", &[("identity", 1)]);
    let checkpoint = ModelCheckpoint::new(&lib, &[-0., 1.], 4, 42, "unit-test").unwrap();
    let bytes = checkpoint.encode().unwrap();
    let restored = ModelCheckpoint::decode(&bytes, &lib, 2).unwrap();
    assert_eq!(restored.parameter_bits, checkpoint.parameter_bits);
    assert!(ModelCheckpoint::decode(&bytes, &lib, 1).is_err());
    let other = library(
        "v2 (pub identity (x:f32) f32 = (+ x 0f))",
        &[("identity", 1)],
    );
    assert!(ModelCheckpoint::decode(&bytes, &other, 2).is_err());
    let mut malformed = checkpoint;
    malformed.parameter_bits[0] = f32::INFINITY.to_bits();
    assert!(ModelCheckpoint::decode(&malformed.encode().unwrap(), &lib, 2).is_err());
}
#[test]
fn shared_budget_never_oversubscribes_concurrent_cpu_lanes() {
    use std::sync::{Arc, Barrier};
    let budget = MemoryBudget::new(80);
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let barrier = barrier.clone();
            let budget = budget.clone();
            scope.spawn(move || {
                let tensor = Tensor::new(&[10], &[1.; 10], &budget);
                barrier.wait();
                assert!(budget.live_bytes() <= 80);
                drop(tensor);
            });
        }
    });
    assert_eq!(budget.live_bytes(), 0);
    assert!(budget.peak_bytes() <= 80);
}
#[test]
fn shipped_fluxr_dual_number_gradient_matches_finite_difference() {
    let lib = ScalarLibrary::load(
        include_bytes!("../examples/native/ml/manifest.json"),
        include_str!("../examples/native/ml/program.flux"),
    )
    .unwrap();
    let mut s = lib.session(Limits::default(), 100, Cancellation::default());
    let parameters = [0.7, 0.3, 1.1, -0.2];
    for direction in 0..4 {
        let gradient = s
            .call(
                "gradient",
                &[
                    0.4,
                    1.8,
                    parameters[0],
                    parameters[1],
                    parameters[2],
                    parameters[3],
                    direction as f32,
                ],
            )
            .unwrap();
        let mut plus = parameters;
        plus[direction] += 0.001;
        let mut minus = parameters;
        minus[direction] -= 0.001;
        let loss_plus = s
            .call("loss", &[0.4, 1.8, plus[0], plus[1], plus[2], plus[3]])
            .unwrap();
        let loss_minus = s
            .call("loss", &[0.4, 1.8, minus[0], minus[1], minus[2], minus[3]])
            .unwrap();
        let numerical = (loss_plus - loss_minus) / 0.002;
        assert!(
            (gradient - numerical).abs() < 0.0002,
            "dir={direction} {gradient} vs {numerical}"
        );
    }
}
#[test]
fn asynchronous_cpu_batch_has_owned_completion_and_cooperative_cancel() {
    use fluxr::cpu_compute::CpuBatchExecutor;
    use std::sync::Arc;
    let lib = Arc::new(library(
        "v2 (pub square (x:f32) f32 = (* x x))",
        &[("square", 1)],
    ));
    let memory = MemoryBudget::new(1024 * 1024);
    let executor = CpuBatchExecutor::new(2).unwrap();
    assert!(CpuBatchExecutor::new(0).is_err());
    let input = Tensor::new(&[3, 1], &[2., 3., 4.], &memory).unwrap();
    let job = executor
        .submit(
            lib.clone(),
            "square".into(),
            input,
            memory.clone(),
            Limits::default(),
            3,
        )
        .unwrap();
    let answer = job.wait().unwrap();
    assert_eq!(answer.values(), &[4., 9., 16.]);
    assert_eq!(executor.active_jobs(), 0);
    let input = Tensor::new(&[32768, 1], &vec![2.; 32768], &memory).unwrap();
    let job = executor
        .submit(
            lib,
            "square".into(),
            input,
            memory.clone(),
            Limits::default(),
            32768,
        )
        .unwrap();
    job.request_cancel();
    assert!(job.cancellation_requested());
    match job.wait() {
        Ok(result) => assert!(result.values().iter().all(|&x| x == 4.)),
        Err(error) => assert_eq!(error.to_string(), "compute cancelled"),
    }
    assert_eq!(executor.active_jobs(), 0);
    drop(answer);
    assert_eq!(memory.live_bytes(), 0);
}
#[test]
fn duplicate_manifest_export_names_are_rejected_as_ambiguous() {
    let (manifest, source) = package("v2 (pub square (x:f32) f32 = (* x x))", &[("square", 1)]);
    let manifest = String::from_utf8(manifest)
        .unwrap()
        .replace("\"square\":1", "\"square\":2,\"square\":1");
    assert!(ScalarLibrary::load(manifest.as_bytes(), &source).is_err());
}
