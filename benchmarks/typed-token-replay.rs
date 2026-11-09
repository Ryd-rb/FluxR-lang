// adapter: fluxr
use fluxr::structured::{Invocation, Limits, Module, Value};

fn call(module: &Module, args: &[Value], trap: &str) -> Option<Value> {
    match module.run(Invocation {
        export: "calc",
        inputs: &[],
        arguments: args,
        limits: Limits::default(),
    }) {
        Ok(value) => Some(value),
        Err(error) => {
            assert_eq!(error.to_string(), trap);
            None
        }
    }
}

// adapter: rust
include!(concat!(env!("FLUXR_TYPED_HOLDOUT"), "/rust-modules.rs"));

// probes
use std::{fs, path::Path};

fn compare(key: &str, probe: impl FnOnce(&Module)) {
    let root = Path::new(env!("FLUXR_TYPED_HOLDOUT"));
    let source = fs::read_to_string(root.join(format!("{key}.fluxr"))).unwrap();
    let module = Module::compile_compact(&source).unwrap();
    let encoded = module.encode();
    let expanded = module.compact_text().unwrap();
    assert_eq!(
        Module::compile_compact(&expanded).unwrap().encode(),
        encoded
    );
    let decoded = Module::decode(&encoded).unwrap();
    probe(&decoded);
    fs::write(root.join(format!("{key}.expanded")), expanded).unwrap();
    println!("{key} passed");
}

const INT_TRAP: &str = "invalid or overflowing integer arithmetic";
const INDEX_TRAP: &str = "list index out of bounds";
const INTS: [i64; 7] = [i64::MIN, -9, -1, 0, 1, 7, i64::MAX];
const WORDS: [u64; 7] = [0, 1, 4, u64::MAX / 4, u64::MAX / 3, u64::MAX - 3, u64::MAX];

fn throttle(m: &Module, rust: impl Fn(bool, bool, u64) -> Option<u64>) {
    for live in [false, true] {
        for hard in [false, true] {
            for n in WORDS {
                assert_eq!(
                    call(
                        m,
                        &[Value::Bool(live), Value::Bool(hard), Value::U64(n)],
                        INT_TRAP
                    ),
                    rust(live, hard, n).map(Value::U64)
                );
            }
        }
    }
}

fn calibration(m: &Module, rust: impl Fn(bool, &[f32]) -> Option<f32>) {
    let payloads = [0x7fa0_0001, 0x8000_0000, 0x7fc0_0312, 0x7f80_0000, 0];
    for bits in [payloads, payloads.map(|bits| bits ^ 0x8000_0000)] {
        for n in 0..=bits.len() {
            let bits = &bits[..n];
            let list = Value::List(
                bits.iter()
                    .copied()
                    .map(Value::F32)
                    .collect::<Vec<_>>()
                    .into(),
            );
            let floats: Vec<_> = bits.iter().copied().map(f32::from_bits).collect();
            for b in [false, true] {
                assert_eq!(
                    call(m, &[Value::Bool(b), list.clone()], INDEX_TRAP),
                    rust(b, &floats).map(|v| Value::F32(v.to_bits()))
                );
            }
        }
    }
}

fn tariff(m: &Module, rust: impl Fn(i64, i64, i64) -> Option<i64>) {
    for per in INTS {
        for base in INTS {
            for n in INTS {
                let record = Value::Record([Value::I64(per), Value::I64(base)].into());
                assert_eq!(
                    call(m, &[record, Value::I64(n)], INT_TRAP),
                    rust(per, base, n).map(Value::I64)
                );
            }
        }
    }
}

fn admission(m: &Module, rust: impl Fn(bool, i64, i64, i64) -> Option<bool>) {
    for enabled in [false, true] {
        for a in INTS {
            for b in INTS {
                for cap in INTS {
                    let args = [
                        Value::Bool(enabled),
                        Value::I64(a),
                        Value::I64(b),
                        Value::I64(cap),
                    ];
                    assert_eq!(
                        call(m, &args, INT_TRAP),
                        rust(enabled, a, b, cap).map(Value::Bool)
                    );
                }
            }
        }
    }
}

fn packet(m: &Module, rust: impl Fn(bool, u64) -> Vec<u64>) {
    for active in [false, true] {
        for n in WORDS {
            let expected = rust(active, n)
                .into_iter()
                .map(Value::U64)
                .collect::<Vec<_>>();
            assert_eq!(
                call(m, &[Value::Bool(active), Value::U64(n)], ""),
                Some(Value::List(expected.into()))
            );
        }
    }
}

fn ack(m: &Module, rust: impl Fn(Option<i64>) -> i64) {
    for n in INTS.into_iter().map(Some).chain([None]) {
        let ack = match n {
            Some(n) => Value::Variant(0, Some(Value::I64(n).into())),
            None => Value::Variant(1, None),
        };
        assert_eq!(call(m, &[ack], ""), Some(Value::I64(rust(n))));
    }
}

fn main() {
    if let Some(path) = std::env::args_os().nth(1) {
        let source = fs::read_to_string(path).unwrap();
        println!(
            "{}",
            Module::compile_compact(&source)
                .err()
                .expect("fault accepted")
        );
    } else {
        replay();
    }
}
