// host: fluxr
use fluxr::structured::{Invocation, Limits, Module, Value};
use fluxr::{Gate, Program, RuleProgram};

fn structured(source: &str) -> Module {
    let module = Module::compile_compact(source).unwrap();
    Module::decode(&module.encode()).unwrap()
}

fn call(module: &Module, arguments: &[Value]) -> Result<Value, fluxr::Error> {
    module.run(Invocation {
        export: "calc",
        inputs: &[],
        arguments,
        limits: Limits::default(),
    })
}

fn checked<T>(
    result: Result<Value, fluxr::Error>,
    trap: &str,
    read: impl FnOnce(Value) -> T,
) -> Option<T> {
    match result {
        Ok(value) => Some(read(value)),
        Err(error) => {
            assert!(error.to_string().contains(trap), "{error}");
            None
        }
    }
}

// host: rust
include!(concat!(env!("FLUXR_HELDOUT"), "/rust-modules.rs"));

// probes
const INT_TRAP: &str = "invalid or overflowing integer arithmetic";
const INDEX_TRAP: &str = "list index out of bounds";
const INTS: [i64; 7] = [i64::MIN, -5, -1, 0, 1, 9, i64::MAX];

fn signed(value: Value) -> i64 {
    let Value::I64(value) = value else {
        panic!("expected i64")
    };
    value
}

fn unsigned(value: Value) -> u64 {
    let Value::U64(value) = value else {
        panic!("expected u64")
    };
    value
}

fn bits(value: Value) -> u32 {
    let Value::F32(bits) = value else {
        panic!("expected f32 bits")
    };
    bits
}

fn checked_quotient(source: &str, rust: fn(i64, i64, i64) -> Option<i64>) {
    let module = structured(source);
    for x in INTS {
        for y in INTS {
            for d in INTS {
                let result = call(&module, &[Value::I64(x), Value::I64(y), Value::I64(d)]);
                assert_eq!(
                    checked(result, INT_TRAP, signed),
                    rust(x, y, d),
                    "{x} {y} {d}"
                );
            }
        }
    }
}

fn ordered_rule(source: &str, rust: fn(i64, i64, i64) -> Option<i64>) {
    let rule = RuleProgram::compile_text(source).unwrap();
    let rule = RuleProgram::decode(&rule.encode()).unwrap();
    for x in INTS {
        for y in INTS {
            for flag in [-1, 0, 1] {
                let result = rule.evaluate(&[x, y, flag]);
                if let Err(error) = &result {
                    assert_eq!(
                        error.to_string(),
                        "rule arithmetic overflow or division by zero"
                    );
                }
                assert_eq!(result.ok(), rust(x, y, flag), "{x} {y} {flag}");
            }
        }
    }
}

fn trace(
    mask: u16,
    invoke: impl FnOnce(&mut dyn FnMut(usize) -> bool) -> bool,
) -> (bool, Vec<usize>) {
    let mut seen = Vec::new();
    let result = invoke(&mut |i| {
        seen.push(i);
        mask & (1 << i) != 0
    });
    (result, seen)
}

fn route_gate(source: &str, rust: fn(&mut dyn FnMut(usize) -> bool) -> bool) {
    let gate = Gate::compile_text(source).unwrap();
    for mask in 0..256 {
        assert_eq!(
            trace(mask, |f| gate.evaluate(f)),
            trace(mask, rust),
            "{mask}"
        );
    }
}

fn translated_pulse(source: &str, rust: fn(u16, u16, u32) -> Option<i32>) {
    let program = Program::compile_text(source).unwrap();
    let program = Program::decode(&program.encode()).unwrap();
    for object in 41..=92 {
        for field in [0, 4, 7, 8] {
            for time in (0..=65).chain([u32::MAX]) {
                assert_eq!(
                    program.sample((object, field), time),
                    rust(object, field, time)
                );
            }
        }
    }
}

fn minimum_bits(source: &str, rust: fn(&[f32]) -> Option<f32>) {
    let module = structured(source);
    let words = [
        0, 0x80000000, 0x7fc00011, 0x7f800001, 0xbf800000, 0xff800000, 0x7f800000,
    ];
    let check = |words: &[u32]| {
        let values = Value::List(
            words
                .iter()
                .copied()
                .map(Value::F32)
                .collect::<Vec<_>>()
                .into(),
        );
        let rust_values: Vec<_> = words.iter().copied().map(f32::from_bits).collect();
        let result = call(&module, &[values]);
        assert_eq!(
            checked(result, INDEX_TRAP, bits),
            rust(&rust_values).map(f32::to_bits)
        );
    };
    check(&[]);
    for a in words {
        check(&[a]);
        for b in words {
            check(&[a, b]);
            for c in words {
                check(&[a, b, c]);
            }
        }
    }
}

fn first_alert(source: &str, rust: fn(&[i64], u64, i64) -> Option<u64>) {
    let module = structured(source);
    for rows in [&[][..], &[0, 1, 2], &[3, 0, 3], &[i64::MAX, i64::MIN]] {
        for n in (0..=4).chain([u64::MAX]) {
            for t in INTS {
                let list = Value::List(
                    rows.iter()
                        .copied()
                        .map(Value::I64)
                        .collect::<Vec<_>>()
                        .into(),
                );
                let result = call(&module, &[list, Value::U64(n), Value::I64(t)]);
                assert_eq!(checked(result, INDEX_TRAP, unsigned), rust(rows, n, t));
            }
        }
    }
}

fn ledger(source: &str, rust: impl Fn(i64, i64, i64) -> Option<(i64, i64)>) {
    let module = structured(source);
    for balance in INTS {
        for watermark in INTS {
            for delta in INTS {
                let record = Value::Record([Value::I64(balance), Value::I64(watermark)].into());
                let actual = checked(
                    call(&module, &[record, Value::I64(delta)]),
                    INT_TRAP,
                    |value| {
                        let Value::Record(fields) = value else {
                            panic!("expected Ledger")
                        };
                        (signed(fields[0].clone()), signed(fields[1].clone()))
                    },
                );
                assert_eq!(actual, rust(balance, watermark, delta));
            }
        }
    }
}

fn main() {
    replay();
}
