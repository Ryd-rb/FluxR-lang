use fluxr::compute::{Kernel, PROFILE, SPECIALIZED_PROFILE, Specialization};
use fluxr::structured::Module;

fn model() -> Module {
    Module::compile_compact(
        "v2 (pub kernel (x:f32 scale:f32 bias:f32) f32 () (ret (+ (* x scale) bias)))",
    )
    .unwrap()
}

#[test]
fn agent_fixture_is_the_same_checked_module_as_compact_source() {
    let agent = Module::compile_agent(include_str!("../examples/vks1-affine.fluxr")).unwrap();
    assert_eq!(agent.encode(), model().encode());
}
fn spec(index: u32, value: f32) -> Specialization {
    Specialization {
        parameter_index: index,
        value_bits: value.to_bits(),
    }
}

#[test]
fn template_reuse_requires_exact_variant_identity_and_original_source() {
    let module = model();
    let a =
        Kernel::compile_specialized(&module, "kernel", 64, &[spec(1, 2.0), spec(2, 1.0)]).unwrap();
    let b =
        Kernel::compile_specialized(&module, "kernel", 64, &[spec(2, 1.0), spec(1, 2.0)]).unwrap();
    let c =
        Kernel::compile_specialized(&module, "kernel", 64, &[spec(1, 4.0), spec(2, 1.0)]).unwrap();
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(a.reflection(), b.reflection());
    assert_eq!(a.bytes(), c.bytes());
    assert_ne!(a.reflection().variant_sha256, c.reflection().variant_sha256);
    assert_eq!(a.source_bytes(), module.encode());
    assert_eq!(a.reflection().profile, SPECIALIZED_PROFILE);
    assert_eq!(a.reflection().input_bindings, 1);
    assert_eq!(a.reflection().output_binding, 1);
    let x: Vec<_> = (0..4097).map(|i| (i - 2048) as f32 / 16.0).collect();
    for (kernel, scale) in [(&a, 2.0), (&c, 4.0)] {
        let got = kernel.cpu_reference(&[&x], x.len()).unwrap();
        for (value, result) in x.iter().zip(got) {
            assert_eq!((value * scale + 1.0).to_bits(), result.to_bits());
        }
    }
}

#[test]
fn admission_refuses_duplicates_nonfinite_unknown_and_all_static_before_execution() {
    let module = model();
    for entries in [
        vec![],
        vec![spec(1, 1.0), spec(1, 2.0)],
        vec![spec(3, 1.0)],
        vec![spec(u32::MAX, 1.0)],
        vec![spec(0, 1.0), spec(1, 2.0), spec(2, 3.0)],
        vec![spec(1, f32::INFINITY)],
        vec![spec(1, f32::NAN)],
        vec![spec(1, 0.0); 8],
    ] {
        assert!(Kernel::compile_specialized(&module, "kernel", 64, &entries).is_err());
    }
    let kernel = Kernel::compile_specialized(&module, "kernel", 64, &[spec(1, 2.0)]).unwrap();
    assert!(kernel.cpu_reference(&[&[1.0]], 1).is_err());
    assert!(kernel.cpu_reference(&[&[], &[]], 1).is_err());
    assert!(kernel.cpu_reference(&[&[1.0], &[f32::NAN]], 1).is_err());
    let unsupported =
        Module::compile_compact("v2 (pub kernel (x:f32 y:f32) f32 () (ret (/ x y)))").unwrap();
    assert!(Kernel::compile_specialized(&unsupported, "kernel", 64, &[spec(1, 2.0)]).is_err());
}

#[test]
fn sparse_specializations_preserve_original_argument_order_and_signed_zero() {
    let module = Module::compile_compact(
        "v2 (pub kernel (a:f32 b:f32 c:f32 d:f32) f32 () (ret (- (+ a b) (* c d))))",
    )
    .unwrap();
    let kernel =
        Kernel::compile_specialized(&module, "kernel", 1, &[spec(0, 8.0), spec(2, 2.0)]).unwrap();
    assert_eq!(
        kernel
            .cpu_reference(&[&[1.0, 2.0], &[3.0, 4.0]], 2)
            .unwrap(),
        [3.0, 2.0]
    );
    let module = Module::compile_compact("v2 (pub kernel (x:f32 z:f32) f32 () (ret z))").unwrap();
    let positive = Kernel::compile_specialized(&module, "kernel", 1, &[spec(1, 0.0)]).unwrap();
    let negative = Kernel::compile_specialized(&module, "kernel", 1, &[spec(1, -0.0)]).unwrap();
    assert_eq!(positive.bytes(), negative.bytes());
    assert_ne!(
        positive.reflection().variant_sha256,
        negative.reflection().variant_sha256
    );
    assert_eq!(
        negative.cpu_reference(&[&[1.0]], 1).unwrap()[0].to_bits(),
        (-0.0_f32).to_bits()
    );
}

#[test]
fn all_parameter_masks_have_checked_vm_equivalence() {
    let module = Module::compile_compact(
        "v2 (pub kernel (a:f32 b:f32 c:f32 d:f32) f32 () (ret (- (+ a b) (* c d))))",
    )
    .unwrap();
    let columns: Vec<Vec<f32>> = (0..4)
        .map(|col| (0..65).map(|i| (i + col) as f32 / 16.0).collect())
        .collect();
    for mask in 1..15 {
        let specs: Vec<_> = (0..4)
            .filter(|j| mask & (1 << j) != 0)
            .map(|j| spec(j, j as f32 + 1.0))
            .collect();
        let inputs: Vec<_> = (0..4)
            .filter(|j| mask & (1 << j) == 0)
            .map(|j| columns[j].as_slice())
            .collect();
        let kernel = Kernel::compile_specialized(&module, "kernel", 3, &specs).unwrap();
        let got = kernel.cpu_reference(&inputs, 65).unwrap();
        for (i, value) in got.into_iter().enumerate() {
            let args: Vec<f32> = (0..4)
                .map(|j| {
                    if mask & (1 << j) != 0 {
                        j as f32 + 1.0
                    } else {
                        columns[j][i]
                    }
                })
                .collect();
            assert_eq!(
                (args[0] + args[1] - args[2] * args[3]).to_bits(),
                value.to_bits()
            );
        }
    }
}

#[test]
fn released_vkf1_has_no_new_serialized_fields_or_specialization_instructions() {
    let kernel = Kernel::compile(&model(), "kernel", 64).unwrap();
    assert_eq!(kernel.reflection().profile, PROFILE);
    let reflection = serde_json::to_value(kernel.reflection()).unwrap();
    assert!(reflection.get("specializations").is_none());
    assert!(reflection.get("variant_sha256").is_none());
    let mut at = 5;
    while at < kernel.words().len() {
        assert_ne!(kernel.words()[at] & 65535, 50);
        at += (kernel.words()[at] >> 16) as usize;
    }
}
