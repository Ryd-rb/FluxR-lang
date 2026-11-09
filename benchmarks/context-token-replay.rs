// adapter: fluxr
use fluxr::structured::{Invocation, Limits, Module, Value};

fn call(module: &Module, lo: i64, hi: i64, d: i64) -> Option<i64> {
    match module.run(Invocation {
        export: "calc",
        inputs: &[],
        arguments: &[
            Value::Record([Value::I64(lo), Value::I64(hi)].into()),
            Value::I64(d),
        ],
        limits: Limits::default(),
    }) {
        Ok(Value::I64(n)) => Some(n),
        Ok(other) => panic!("expected i64, got {other:?}"),
        Err(error) => {
            assert_eq!(
                error.to_string(),
                "invalid or overflowing integer arithmetic"
            );
            None
        }
    }
}

// adapter: rust
include!(concat!(env!("FLUXR_CONTEXT_REPLAY"), "/rust-modules.rs"));

// probes
use std::{fs, path::Path};

fn compare(key: &str, rust: impl Fn(i64, i64, i64) -> Option<i64>) -> Vec<Option<i64>> {
    let root = Path::new(env!("FLUXR_CONTEXT_REPLAY"));
    let source = fs::read_to_string(root.join(format!("{key}.fluxr"))).unwrap();
    let module = Module::compile_compact(&source).unwrap();
    let encoded = module.encode();
    let expanded = module.compact_text().unwrap();
    assert_eq!(
        Module::compile_compact(&expanded).unwrap().encode(),
        encoded
    );
    let module = Module::decode(&encoded).unwrap();
    let mut outputs = Vec::new();
    let values = [i64::MIN, -9, -1, 0, 1, 7, i64::MAX];
    for lo in values {
        for hi in values {
            for d in values {
                let want = rust(lo, hi, d);
                assert_eq!(call(&module, lo, hi, d), want, "{key}: {lo}, {hi}, {d}");
                outputs.push(want);
            }
        }
    }
    assert!(outputs.iter().any(Option::is_some));
    assert!(outputs.iter().any(Option::is_none));
    println!("{key} passed {} inputs", outputs.len());
    outputs
}

fn main() {
    if let Some(path) = std::env::args_os().nth(1) {
        let source = fs::read_to_string(path).unwrap();
        println!("{}", Module::compile_compact(&source).unwrap_err());
    } else {
        replay();
    }
}
