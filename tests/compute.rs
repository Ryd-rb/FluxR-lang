use fluxr::{compute::Kernel, structured::Module};

fn module(body: &str) -> Module {
    Module::compile_compact(&format!(
        "v2 (pub kernel (x:f32 y:f32) f32 () (ret {body}))"
    ))
    .unwrap()
}
#[test]
fn immutable_spirv_and_reflection_reproduce_exactly() {
    let module = module("(+ (* x 2f) y)");
    let a = Kernel::compile(&module, "kernel", 64).unwrap();
    let b = Kernel::compile(&Module::decode(&module.encode()).unwrap(), "kernel", 64).unwrap();
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(a.reflection(), b.reflection());
    assert_eq!(a.reflection().array_stride_bytes, 4);
    assert_eq!(a.reflection().workgroup, [64, 1, 1]);
    assert_eq!(a.reflection().arithmetic_operations, 2);
    assert_ne!(
        a.bytes(),
        Kernel::compile(&module, "kernel", 32).unwrap().bytes()
    );
}
#[test]
fn existing_vm_is_the_numerical_reference() {
    let kernel = Kernel::compile(&module("(+ (* x 2f) y)"), "kernel", 64).unwrap();
    let x: Vec<_> = (0..4097).map(|i| (i - 2048) as f32 / 16.0).collect();
    let y: Vec<_> = (0..4097).map(|i| (i % 127) as f32 / 32.0).collect();
    let got = kernel.cpu_reference(&[&x, &y], 4097).unwrap();
    for ((x, y), got) in x.iter().zip(y).zip(got) {
        assert_eq!((x * 2.0 + y).to_bits(), got.to_bits());
    }
}
#[test]
fn lowering_rejects_unsupported_semantics_and_workgroup_budgets() {
    for expr in ["(/ x y)", "(sqrt x)", "0x7fc00000f", "0x7f800000f"] {
        assert!(
            Kernel::compile(&module(expr), "kernel", 64).is_err(),
            "{expr}"
        );
    }
    for size in [0, 257, u32::MAX] {
        assert!(Kernel::compile(&module("x"), "kernel", size).is_err());
    }
    let looped = Module::compile_compact(
        "v2 (pub kernel (x:f32) f32 (i:i64) (= i 0) (while (< i 1) ((= i (+ i 1)))) (ret x))",
    )
    .unwrap();
    assert!(Kernel::compile(&looped, "kernel", 64).is_err());
    let host =
        Module::compile_compact("v2 (inputs a:f32) (pub kernel (x:f32) f32 () (ret (+ x $a)))")
            .unwrap();
    assert!(Kernel::compile(&host, "kernel", 64).is_err());
}
#[test]
fn shape_finiteness_and_element_budgets_fail_before_execution() {
    let kernel = Kernel::compile(&module("(+ x y)"), "kernel", 64).unwrap();
    assert!(kernel.cpu_reference(&[&[1.0], &[]], 10).is_err());
    assert!(kernel.cpu_reference(&[&[], &[]], 10).is_err());
    assert!(kernel.cpu_reference(&[&[f32::NAN], &[1.0]], 10).is_err());
    assert!(kernel.cpu_reference(&[&[1.0], &[1.0]], 0).is_err());
    assert!(kernel.cpu_reference(&[&[1.0]], 10).is_err());
}
#[test]
fn instruction_stream_is_bounded_and_no_contraction_is_explicit() {
    let kernel = Kernel::compile(&module("(- (* x y) (+ x y))"), "kernel", 63).unwrap();
    let words = kernel.words();
    assert_eq!(words[0], 0x07230203);
    let mut i = 5;
    let mut no_contraction = 0;
    let mut guards = 0;
    while i < words.len() {
        let width = (words[i] >> 16) as usize;
        assert!(width > 0 && i + width <= words.len());
        if words[i] & 65535 == 71 && words[i + 2] == 42 {
            no_contraction += 1;
        }
        if words[i] & 65535 == 176 {
            guards += 1;
        }
        i += width;
    }
    assert_eq!(no_contraction, 3);
    assert_eq!(guards, 1);
}
