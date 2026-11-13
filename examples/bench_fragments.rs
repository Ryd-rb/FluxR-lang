//! Checked fragment coverage with separate individual-call and batch observations.
//! This is a finite empirical experiment, not a universal speed or safety proof.
//! `fluxr_public_baseline` adapts this probe to the older public Module API only.
#![allow(unexpected_cfgs)]

#[cfg(not(fluxr_public_baseline))]
use fluxr::structured::{ExecutionRoute, SourceDocument};
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

const VALIDATION_ROWS: usize = 64;
const WARMUP: usize = 256;
const LIFECYCLE: usize = 11;

type Outcome = Result<Value, String>;

struct Row {
    inputs: Vec<Value>,
    arguments: Vec<Value>,
    limits: Limits,
    expected: Outcome,
}

impl Row {
    fn new(arguments: Vec<Value>, expected: Value) -> Self {
        Self {
            inputs: vec![],
            arguments,
            limits: Limits::default(),
            expected: Ok(expected),
        }
    }

    fn failure(arguments: Vec<Value>, error: &str) -> Self {
        Self {
            expected: Err(error.to_owned()),
            ..Self::new(arguments, Value::Unit)
        }
    }

    fn invocation(&self) -> Invocation<'_> {
        Invocation {
            export: "run",
            inputs: black_box(&self.inputs),
            arguments: black_box(&self.arguments),
            limits: black_box(self.limits),
        }
    }

    fn identity(&self) -> Json {
        json!({"inputs":self.inputs,"arguments":self.arguments,
            "limits":self.limits,"expected":self.expected})
    }
}

struct Case {
    name: &'static str,
    coverage: &'static [&'static str],
    source: &'static str,
    row: fn(usize) -> Row,
    native_required: bool,
}

fn signed(n: usize) -> i64 {
    n as i64 % 31 - 15
}

fn list(values: &[i64]) -> Value {
    Value::List(
        values
            .iter()
            .copied()
            .map(Value::I64)
            .collect::<Vec<_>>()
            .into(),
    )
}

fn record(values: &[i64]) -> Value {
    Value::Record(
        values
            .iter()
            .copied()
            .map(Value::I64)
            .collect::<Vec<_>>()
            .into(),
    )
}

// Expected outputs are ordinary Rust calculations over the declared bounded
// inputs, not values harvested from the evaluator or the native planner.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "scalar_0",
            coverage: &["i64", "zero_argument"],
            source: "v2 (pub run () i64 = 42)",
            row: |_| Row::new(vec![], Value::I64(42)),
            native_required: true,
        },
        Case {
            name: "scalar_1",
            coverage: &["i64", "checked_add"],
            source: "v2 (pub run (x:i64) i64 = (+ x 17))",
            row: |n| {
                Row::new(
                    vec![Value::I64(signed(n))],
                    Value::I64(signed(n).checked_add(17).unwrap()),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_2",
            coverage: &["i64", "checked_mul", "checked_rem"],
            source: "v2 (pub run (x:i64 y:i64) i64 = (% (+ (* x y) 17) 997))",
            row: |n| {
                let (x, y) = (signed(n), (n % 13 + 1) as i64);
                Row::new(
                    vec![Value::I64(x), Value::I64(y)],
                    Value::I64(
                        x.checked_mul(y)
                            .unwrap()
                            .checked_add(17)
                            .unwrap()
                            .checked_rem(997)
                            .unwrap(),
                    ),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_3",
            coverage: &["i64", "checked_sub", "checked_div"],
            source: "v2 (pub run (x:i64 y:i64 z:i64) i64 = (- (/ x y) z))",
            row: |n| {
                let (x, y, z) = (signed(n), (n % 7 + 1) as i64, (n % 5) as i64);
                Row::new(
                    vec![Value::I64(x), Value::I64(y), Value::I64(z)],
                    Value::I64(x.checked_div(y).unwrap().checked_sub(z).unwrap()),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_4",
            coverage: &["i64", "four_argument"],
            source: "v2 (pub run (a:i64 b:i64 c:i64 d:i64) i64 = (+ (* a b) (- c d)))",
            row: |n| {
                let x = signed(n);
                Row::new(
                    vec![Value::I64(x), Value::I64(3), Value::I64(8), Value::I64(2)],
                    Value::I64(x.checked_mul(3).unwrap().checked_add(6).unwrap()),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_5",
            coverage: &["i64", "generic_boundary"],
            source: "v2 (pub run (a:i64 b:i64 c:i64 d:i64 e:i64) i64 = (+ (+ (+ (+ a b) c) d) e))",
            row: |n| {
                Row::new(
                    vec![
                        Value::I64(signed(n)),
                        Value::I64(1),
                        Value::I64(2),
                        Value::I64(3),
                        Value::I64(4),
                    ],
                    Value::I64(signed(n) + 10),
                )
            },
            native_required: true,
        },
        Case {
            name: "split_inputs",
            coverage: &["input_snapshot", "i64", "four_lane"],
            source: "v2 (inputs base:i64 step:i64) (pub run (x:i64 y:i64) i64 = (+ (+ (input base) (* (input step) x)) y))",
            row: |n| {
                let mut r = Row::new(
                    vec![Value::I64(signed(n)), Value::I64(5)],
                    Value::I64(12 + 3 * signed(n)),
                );
                r.inputs = vec![Value::I64(7), Value::I64(3)];
                r
            },
            native_required: true,
        },
        Case {
            name: "branch",
            coverage: &["if", "early_return", "comparison"],
            source: "v2 (pub run (x:i64) i64 () (if (< x 0) ((ret (- 0 x))) ((ret (+ x 1)))))",
            row: |n| {
                let x = signed(n);
                Row::new(
                    vec![Value::I64(x)],
                    Value::I64(if x < 0 { -x } else { x + 1 }),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_compare_bool",
            coverage: &["i64", "comparison", "bool_result", "scalar_boundary"],
            source: "v2 (pub run (x:i64 y:i64) bool = (< x y))",
            row: |n| {
                let (x, y) = (signed(n), signed(63 - n));
                Row::new(vec![Value::I64(x), Value::I64(y)], Value::Bool(x < y))
            },
            native_required: true,
        },
        Case {
            name: "scalar_bool_identity",
            coverage: &["bool", "identity", "scalar_boundary"],
            source: "v2 (pub run (flag:bool) bool = flag)",
            row: |n| Row::new(vec![Value::Bool(n % 2 == 0)], Value::Bool(n % 2 == 0)),
            native_required: true,
        },
        Case {
            name: "scalar_bool_equal",
            coverage: &["bool", "equality", "scalar_boundary"],
            source: "v2 (pub run (a:bool b:bool) bool = (== a b))",
            row: |n| {
                let (a, b) = (n % 2 == 0, n % 4 < 2);
                Row::new(vec![Value::Bool(a), Value::Bool(b)], Value::Bool(a == b))
            },
            native_required: true,
        },
        Case {
            name: "scalar_mixed_branch",
            coverage: &["bool", "i64", "branch", "scalar_boundary"],
            source: "v2 (pub run (flag:bool x:i64 y:i64) i64 () (if flag ((ret (+ x 3))) ((ret (- y 2)))))",
            row: |n| {
                let (flag, x, y) = (n % 2 == 0, signed(n), signed(63 - n));
                Row::new(
                    vec![Value::Bool(flag), Value::I64(x), Value::I64(y)],
                    Value::I64(if flag { x + 3 } else { y - 2 }),
                )
            },
            native_required: true,
        },
        Case {
            name: "scalar_split_bool",
            coverage: &["bool", "i64", "input_snapshot", "scalar_boundary"],
            source: "v2 (inputs flag:bool base:i64) (pub run (delta:i64) i64 () (if (input flag) ((ret (+ (input base) delta))) ((ret (- (input base) delta)))))",
            row: |n| {
                let (flag, base, delta) = (n % 2 == 0, signed(n), (n % 7) as i64);
                let mut row = Row::new(
                    vec![Value::I64(delta)],
                    Value::I64(if flag { base + delta } else { base - delta }),
                );
                row.inputs = vec![Value::Bool(flag), Value::I64(base)];
                row
            },
            native_required: true,
        },
        Case {
            name: "scalar_mixed_5",
            coverage: &["bool", "i64", "five_lanes", "generic_boundary_control"],
            source: "v2 (pub run (flag:bool x:i64 y:i64 z:i64 w:i64) i64 () (if flag ((ret (+ (+ x y) (+ z w)))) ((ret (- (- x y) (+ z w))))))",
            row: |n| {
                let (flag, x) = (n % 2 == 0, signed(n));
                Row::new(
                    vec![
                        Value::Bool(flag),
                        Value::I64(x),
                        Value::I64(2),
                        Value::I64(3),
                        Value::I64(4),
                    ],
                    Value::I64(if flag { x + 9 } else { x - 9 }),
                )
            },
            native_required: true,
        },
        Case {
            name: "boolean_short_circuit",
            coverage: &["bool", "and", "or", "not", "skipped_trap"],
            source: "v2 (pub run (x:i64) bool = (|| (&& (! (< x 0)) (<= x 7)) (&& false (== (/ x 0) 0))))",
            row: |n| {
                let x = signed(n);
                Row::new(vec![Value::I64(x)], Value::Bool((0..=7).contains(&x)))
            },
            native_required: false,
        },
        Case {
            name: "bounded_loop",
            coverage: &["while", "local_assignment", "dynamic_metering"],
            source: "v2 (pub run (n:i64) i64 (acc:i64) (= acc 0) (while (< 0 n) ((= acc (+ acc n)) (= n (- n 1)))) (ret acc))",
            row: |n| {
                let n = (n % 16) as i64;
                Row::new(vec![Value::I64(n)], Value::I64(n * (n + 1) / 2))
            },
            native_required: true,
        },
        Case {
            name: "loop_exits",
            coverage: &["while", "break", "continue"],
            source: "v2 (pub run (n:i64) i64 (acc:i64) (= acc 0) (while (< 0 n) ((= n (- n 1)) (if (== n 2) ((continue))) (if (== n 0) ((break))) (= acc (+ acc n)))) (ret acc))",
            row: |n| {
                let n = (n % 12) as i64;
                Row::new(
                    vec![Value::I64(n)],
                    Value::I64((1..n).filter(|v| *v != 2).sum()),
                )
            },
            native_required: false,
        },
        Case {
            name: "loop_nested",
            coverage: &["while", "nested_loops", "dynamic_metering", "zero_trip"],
            source: "v2 (pub run (n:i64) i64 (i:i64 j:i64 sum:i64) (= i 0) (= sum 0) (while (< i n) ((= j 0) (while (< j i) ((= sum (+ sum j)) (= j (+ j 1)))) (= i (+ i 1)))) (ret sum))",
            row: |n| {
                let n = (n % 13) as i64;
                let expected = (0..n).map(|i| (0..i).sum::<i64>()).sum();
                Row::new(vec![Value::I64(n)], Value::I64(expected))
            },
            native_required: true,
        },
        Case {
            name: "loop_branch",
            coverage: &["while", "branch", "checked_remainder", "dynamic_metering"],
            source: "v2 (pub run (n:i64) i64 (sum:i64) (= sum 0) (while (< 0 n) ((if (== (% n 2) 0) ((= sum (+ sum n))) ((= sum (- sum n)))) (= n (- n 1)))) (ret sum))",
            row: |n| {
                let n = (n % 13) as i64;
                let expected = (1..=n).map(|i| if i % 2 == 0 { i } else { -i }).sum();
                Row::new(vec![Value::I64(n)], Value::I64(expected))
            },
            native_required: true,
        },
        Case {
            name: "loop_acyclic_calls",
            coverage: &["while", "acyclic_helper", "checked_mul", "dynamic_metering"],
            source: "v2 (fn twist (x:i64) i64 = (+ (* x 3) 1)) (pub run (n:i64) i64 (sum:i64) (= sum 0) (while (< 0 n) ((= sum (+ sum (call twist n))) (= n (- n 1)))) (ret sum))",
            row: |n| {
                let n = (n % 13) as i64;
                let expected = (1..=n)
                    .map(|i| i.checked_mul(3).unwrap().checked_add(1).unwrap())
                    .sum();
                Row::new(vec![Value::I64(n)], Value::I64(expected))
            },
            native_required: true,
        },
        Case {
            name: "looping_callee",
            coverage: &["call", "looping_helper", "dynamic_metering", "zero_trip"],
            source: "v2 (fn count (n:i64) i64 (sum:i64) (= sum 0) (while (< 0 n) ((= sum (+ sum n)) (= n (- n 1)))) (ret sum)) (pub run (n:i64) i64 = (+ (call count n) 1))",
            row: |n| {
                let n = (n % 13) as i64;
                Row::new(vec![Value::I64(n)], Value::I64((1..=n).sum::<i64>() + 1))
            },
            native_required: true,
        },
        Case {
            name: "loop_record_update",
            coverage: &[
                "while",
                "record",
                "copy_on_write",
                "zero_trip_ownership",
                "dynamic_metering",
            ],
            source: "v2 (rec R value:i64) (pub run (s:R n:i64 delta:i64) R () (while (< 0 n) ((= s.value (+ s.value delta)) (= n (- n 1)))) (ret s))",
            row: |n| {
                let (initial, iterations) = (signed(n), (n % 13) as i64);
                Row::new(
                    vec![record(&[initial]), Value::I64(iterations), Value::I64(3)],
                    record(&[initial
                        .checked_add(iterations.checked_mul(3).unwrap())
                        .unwrap()]),
                )
            },
            native_required: true,
        },
        Case {
            name: "acyclic_calls",
            coverage: &["call", "nested_calls", "checked_mul"],
            source: "v2 (fn square (x:i64) i64 = (+ (* x x) 3)) (pub run (x:i64 y:i64) i64 = (+ (call square x) (call square y)))",
            row: |n| {
                let (x, y) = (signed(n), (n % 7) as i64);
                Row::new(
                    vec![Value::I64(x), Value::I64(y)],
                    Value::I64(x * x + y * y + 6),
                )
            },
            native_required: true,
        },
        Case {
            name: "recursive_calls",
            coverage: &["recursion", "call_frames", "portable_fallback"],
            source: "v2 (pub run (n:i64) i64 () (if (== n 0) ((ret 0)) ((ret (+ n (call run (- n 1)))))))",
            row: |n| {
                let n = (n % 12) as i64;
                Row::new(vec![Value::I64(n)], Value::I64(n * (n + 1) / 2))
            },
            native_required: false,
        },
        Case {
            name: "record_read",
            coverage: &["record", "unchanged_ownership"],
            source: "v2 (rec R x:i64 y:i64) (pub run (s:R) R = s)",
            row: |n| Row::new(vec![record(&[signed(n), 7])], record(&[signed(n), 7])),
            native_required: true,
        },
        Case {
            name: "record_update",
            coverage: &["record", "field_assignment", "copy_on_write"],
            source: "v2 (rec R x:i64 y:i64) (pub run (s:R d:i64) R () (= s.x (+ s.x d)) (ret s))",
            row: |n| {
                Row::new(
                    vec![record(&[signed(n), 7]), Value::I64(3)],
                    record(&[signed(n) + 3, 7]),
                )
            },
            native_required: true,
        },
        Case {
            name: "record_construct",
            coverage: &["record_construction", "portable_fallback"],
            source: "v2 (rec R x:i64 y:i64) (pub run (x:i64) R = (new R x (+ x 1)))",
            row: |n| {
                Row::new(
                    vec![Value::I64(signed(n))],
                    record(&[signed(n), signed(n) + 1]),
                )
            },
            native_required: false,
        },
        Case {
            name: "nested_record",
            coverage: &["nested_record", "field", "portable_fallback"],
            source: "v2 (rec R x:i64) (rec Outer r:R) (pub run (s:Outer) i64 = (field (field s r) x))",
            row: |n| {
                Row::new(
                    vec![Value::Record(vec![record(&[signed(n)])].into())],
                    Value::I64(signed(n)),
                )
            },
            native_required: false,
        },
        Case {
            name: "method",
            coverage: &["nominal_method", "record", "call_order"],
            source: "v2 (rec R x:i64) (fn R.shift (self:R d:i64) i64 = (+ self.x d)) (pub run (s:R d:i64) i64 = (m s shift d))",
            row: |n| {
                Row::new(
                    vec![record(&[signed(n)]), Value::I64(9)],
                    Value::I64(signed(n) + 9),
                )
            },
            native_required: false,
        },
        Case {
            name: "unsigned",
            coverage: &["u64", "checked_arithmetic", "portable_fallback"],
            source: "v2 (pub run (x:u64) u64 = (% (+ (* x x) 17u) 997u))",
            row: |n| {
                let x = n as u64;
                Row::new(vec![Value::U64(x)], Value::U64((x * x + 17) % 997))
            },
            native_required: false,
        },
        Case {
            name: "binary32",
            coverage: &["f32", "strict_rounding", "sqrt", "abs"],
            source: "v2 (pub run (x:f32 y:f32) f32 = (sqrt (abs (+ (* x y) 1f))))",
            row: |n| {
                let (x, y) = (signed(n) as f32 * 0.25, 1.5_f32);
                let product = x * y;
                let sum = product + 1.0;
                Row::new(
                    vec![Value::F32(x.to_bits()), Value::F32(y.to_bits())],
                    Value::F32(sum.abs().sqrt().to_bits()),
                )
            },
            native_required: false,
        },
        Case {
            name: "float_conversions",
            coverage: &["i64tof", "tof", "ftoi64"],
            source: "v2 (pub run (x:i64 y:u64) i64 = (ftoi64 (+ (i64tof x) (tof y))))",
            row: |n| {
                Row::new(
                    vec![Value::I64(signed(n)), Value::U64(7)],
                    Value::I64(signed(n) + 7),
                )
            },
            native_required: false,
        },
        Case {
            name: "list_index",
            coverage: &["list", "index", "input_validation"],
            source: "v2 (pub run (xs:[i64] i:u64) i64 = (at xs i))",
            row: |n| {
                Row::new(
                    vec![list(&[3, 7, 11, 19]), Value::U64((n % 4) as u64)],
                    Value::I64([3, 7, 11, 19][n % 4]),
                )
            },
            native_required: false,
        },
        Case {
            name: "list_construct",
            coverage: &["list_literal", "checked_arithmetic", "allocation"],
            source: "v2 (pub run (x:i64) [i64] = (list i64 x (+ x 1) (+ x 2)))",
            row: |n| {
                Row::new(
                    vec![Value::I64(signed(n))],
                    list(&[signed(n), signed(n) + 1, signed(n) + 2]),
                )
            },
            native_required: false,
        },
        Case {
            name: "list_empty",
            coverage: &["list", "empty", "empty_and_nonempty_inputs"],
            source: "v2 (pub run (xs:[i64]) bool = (empty xs))",
            row: |n| {
                Row::new(
                    vec![if n % 2 == 0 {
                        list(&[])
                    } else {
                        list(&[signed(n)])
                    }],
                    Value::Bool(n % 2 == 0),
                )
            },
            native_required: false,
        },
        Case {
            name: "list_copy",
            coverage: &["list", "push", "pop", "last", "empty", "copy_on_write"],
            source: "v2 (pub run (xs:[i64] x:i64) [i64] () (if (! (empty xs)) ((pop xs))) (push xs x) (push xs (last xs)) (ret xs))",
            row: |n| {
                Row::new(
                    vec![list(&[3, 7, 11]), Value::I64(signed(n))],
                    list(&[3, 7, signed(n), signed(n)]),
                )
            },
            native_required: false,
        },
        Case {
            name: "nested_list",
            coverage: &["nested_list", "index", "portable_fallback"],
            source: "v2 (pub run (xs:[[i64]] i:u64) i64 = (at (at xs i) 1u))",
            row: |n| {
                Row::new(
                    vec![
                        Value::List(vec![list(&[1, 3]), list(&[5, 7])].into()),
                        Value::U64((n % 2) as u64),
                    ],
                    Value::I64(if n % 2 == 0 { 3 } else { 7 }),
                )
            },
            native_required: false,
        },
        Case {
            name: "variant_payload",
            coverage: &["enum", "is_variant", "payload", "tag"],
            source: "v2 (enum E Empty Number:i64) (pub run (v:#E) #E () (if (is v Number) ((ret (tag E.Number (+ (payload v Number) 1))))) (ret (tag E.Empty)))",
            row: |n| {
                let (arg, value) = if n % 2 == 0 {
                    (Value::Variant(0, None), Value::Variant(0, None))
                } else {
                    (
                        Value::Variant(1, Some(Value::I64(signed(n)).into())),
                        Value::Variant(1, Some(Value::I64(signed(n) + 1).into())),
                    )
                };
                Row::new(vec![arg], value)
            },
            native_required: false,
        },
        Case {
            name: "binary32_scan",
            coverage: &["minidx", "nan", "signed_zero", "tie", "precompiled_scan"],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |n| {
                let bits: &[u32] = match n % 4 {
                    0 => &[0x7fc00000, 0x3f800000, 0xbf800000],
                    1 => &[0, 0x80000000, 0],
                    2 => &[0x40000000, 0x3f800000, 0x3f800000],
                    _ => &[],
                };
                let expected = match n % 4 {
                    0 => 0,
                    1 => 0,
                    2 => 1,
                    _ => 0,
                };
                Row::new(
                    vec![Value::List(
                        bits.iter()
                            .copied()
                            .map(Value::F32)
                            .collect::<Vec<_>>()
                            .into(),
                    )],
                    Value::U64(expected),
                )
            },
            native_required: false,
        },
        Case {
            name: "unit",
            coverage: &["unit", "return"],
            source: "v2 (pub run () unit = unit)",
            row: |_| Row::new(vec![], Value::Unit),
            native_required: false,
        },
        Case {
            name: "scan_empty",
            coverage: &["minidx", "empty_list", "precompiled_scan"],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |_| Row::new(vec![Value::List(vec![].into())], Value::U64(0)),
            native_required: false,
        },
        Case {
            name: "scan_short",
            coverage: &["minidx", "eight_elements", "precompiled_scan"],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |n| {
                let index = n % 8;
                let values = (0..8)
                    .map(|i| {
                        Value::F32(if i == index {
                            (-1.0_f32).to_bits()
                        } else {
                            (i as f32).to_bits()
                        })
                    })
                    .collect::<Vec<_>>();
                Row::new(vec![Value::List(values.into())], Value::U64(index as u64))
            },
            native_required: false,
        },
        Case {
            name: "scan_long",
            coverage: &["minidx", "1024_elements", "precompiled_scan"],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |n| {
                let index = (n * 17) % 1024;
                let values = (0..1024)
                    .map(|i| {
                        Value::F32(if i == index {
                            (-1.0_f32).to_bits()
                        } else {
                            (i as f32).to_bits()
                        })
                    })
                    .collect::<Vec<_>>();
                Row::new(vec![Value::List(values.into())], Value::U64(index as u64))
            },
            native_required: false,
        },
        Case {
            name: "scan_descending",
            coverage: &[
                "minidx",
                "1024_elements",
                "descending",
                "minimum_changes_every_element",
                "precompiled_scan",
            ],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |n| {
                // Every adjacent finite value decreases by exactly one. The
                // last position is the minimum independently of VM lowering.
                let values = (0..1024)
                    .map(|i| Value::F32(((1023 - i + n % 16) as f32).to_bits()))
                    .collect::<Vec<_>>();
                Row::new(vec![Value::List(values.into())], Value::U64(1023))
            },
            native_required: false,
        },
        Case {
            name: "scan_mixed",
            coverage: &[
                "minidx",
                "1024_elements",
                "mixed_signs",
                "subnormal",
                "nan",
                "infinity",
                "signed_zero",
                "tie",
                "varying_minimum_position",
                "precompiled_scan",
            ],
            source: "v2 (pub run (xs:[f32]) u64 = (minidx xs))",
            row: |n| {
                // Known finite entries are in [-64, 64]. NaNs do not win a
                // strict comparison, and only the planted minima can be
                // negative infinity or -2048. Inputs are constructed from bits
                // without executing floating operations on signaling NaNs.
                const PALETTE: [u32; 16] = [
                    0x3f80_0000,
                    0xbf80_0000,
                    0,
                    0x8000_0000,
                    1,
                    0x8000_0001,
                    0x3f00_0000,
                    0xbf00_0000,
                    0x7f80_0000,
                    0x7fc0_0001,
                    0xffc0_0001,
                    0x7f80_0001,
                    0xff80_0001,
                    0x4280_0000,
                    0xc280_0000,
                    0x4000_0000,
                ];
                let mut values = (0..1024)
                    .map(|i| Value::F32(PALETTE[(i * 13 + n * 7) % PALETTE.len()]))
                    .collect::<Vec<_>>();
                let index = (n * 17) % 1023 + 1;
                let minimum = if n % 4 == 2 { 0xc500_0000 } else { 0xff80_0000 };
                values[0] = Value::F32(0x4280_0000);
                values[index] = Value::F32(minimum);
                if index + 1 < values.len() {
                    values[index + 1] = Value::F32(minimum); // First equal minimum wins.
                }
                let expected = match n % 4 {
                    0 => {
                        values[0] = Value::F32(0x7fc0_0001); // First quiet NaN is retained.
                        0
                    }
                    1 => {
                        values[0] = Value::F32(0xff80_0001); // Likewise a negative signaling NaN.
                        0
                    }
                    _ => index,
                };
                Row::new(
                    vec![Value::List(values.into())],
                    Value::U64(expected as u64),
                )
            },
            native_required: false,
        },
        Case {
            name: "overflow",
            coverage: &["trap", "checked_overflow", "native_decline"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |_| {
                Row::failure(
                    vec![Value::I64(i64::MAX)],
                    "invalid or overflowing integer arithmetic",
                )
            },
            native_required: false,
        },
        Case {
            name: "division_trap",
            coverage: &["trap", "division_by_zero", "first_failure"],
            source: "v2 (pub run (x:i64) i64 = (/ x 0))",
            row: |n| {
                Row::failure(
                    vec![Value::I64(signed(n))],
                    "invalid or overflowing integer arithmetic",
                )
            },
            native_required: false,
        },
        Case {
            name: "uninitialized_trap",
            coverage: &["trap", "uninitialized_local", "first_failure"],
            source: "v2 (pub run (x:i64) i64 (a:i64) (ret (+ a (/ x 0))))",
            row: |n| Row::failure(vec![Value::I64(signed(n))], "read of uninitialized local"),
            native_required: false,
        },
        Case {
            name: "index_trap",
            coverage: &["trap", "bounds", "list"],
            source: "v2 (pub run (xs:[i64]) i64 = (at xs 8u))",
            row: |_| Row::failure(vec![list(&[1, 2])], "list index out of bounds"),
            native_required: false,
        },
        Case {
            name: "last_empty_trap",
            coverage: &["trap", "empty_list", "last"],
            source: "v2 (pub run (xs:[i64]) i64 = (last xs))",
            row: |_| Row::failure(vec![list(&[])], "last from empty list"),
            native_required: false,
        },
        Case {
            name: "pop_empty_trap",
            coverage: &["trap", "empty_list", "pop"],
            source: "v2 (pub run (xs:[i64]) [i64] () (pop xs) (ret xs))",
            row: |_| Row::failure(vec![list(&[])], "pop from empty list"),
            native_required: false,
        },
        Case {
            name: "variant_payload_trap",
            coverage: &["trap", "enum", "payload_tag_mismatch"],
            source: "v2 (enum E Empty Number:i64) (pub run (v:#E) i64 = (payload v Number))",
            row: |_| {
                Row::failure(
                    vec![Value::Variant(0, None)],
                    "enum payload does not match variant",
                )
            },
            native_required: false,
        },
        Case {
            name: "float_nonfinite_to_int",
            coverage: &["f32", "ftoi64", "nan", "infinity", "saturating_conversion"],
            source: "v2 (pub run (x:f32) i64 = (ftoi64 x))",
            row: |n| {
                let (bits, expected) = match n % 4 {
                    0 => (0x7fc0_0001, 0),
                    1 => (0xff80_0001, 0),
                    2 => (0x7f80_0000, i64::MAX),
                    _ => (0xff80_0000, i64::MIN),
                };
                Row::new(vec![Value::F32(bits)], Value::I64(expected))
            },
            native_required: false,
        },
        Case {
            name: "float_range_to_int",
            coverage: &["f32", "ftoi64", "range_boundary", "saturating_conversion"],
            source: "v2 (pub run (x:f32) i64 = (ftoi64 x))",
            row: |n| {
                // Literal bit patterns and exact integer expectations make the
                // oracle independent of the VM's Rust float-to-integer cast.
                let (bits, expected) = match n % 4 {
                    0 => (0x5f00_0000, i64::MAX), // +2^63 saturates.
                    1 => (0xdf00_0001, i64::MIN), // Below -2^63 saturates.
                    2 => (0x5eff_ffff, 9_223_371_487_098_961_920),
                    _ => (0xdf00_0000, i64::MIN), // Exactly -2^63.
                };
                Row::new(vec![Value::F32(bits)], Value::I64(expected))
            },
            native_required: false,
        },
        Case {
            name: "unsigned_overflow",
            coverage: &["u64", "trap", "checked_overflow"],
            source: "v2 (pub run (x:u64) u64 = (+ x 1u))",
            row: |_| {
                Row::failure(
                    vec![Value::U64(u64::MAX)],
                    "invalid or overflowing integer arithmetic",
                )
            },
            native_required: false,
        },
        Case {
            name: "fuel_limit",
            coverage: &["logical_fuel", "limit_failure", "portable_replay"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |n| {
                let mut r = Row::failure(
                    vec![Value::I64(signed(n))],
                    "module instruction budget exhausted",
                );
                r.limits.instructions = 1;
                r
            },
            native_required: false,
        },
        Case {
            name: "cell_limit",
            coverage: &["logical_cells", "limit_failure", "portable_replay"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |n| {
                let mut r =
                    Row::failure(vec![Value::I64(signed(n))], "module cell budget exhausted");
                r.limits.cells = 0;
                r
            },
            native_required: false,
        },
        Case {
            name: "frame_limit",
            coverage: &["logical_frames", "limit_failure", "portable_replay"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |n| {
                let mut r =
                    Row::failure(vec![Value::I64(signed(n))], "module frame limit exceeded");
                r.limits.frames = 0;
                r
            },
            native_required: false,
        },
        Case {
            name: "wrong_type",
            coverage: &["boundary_types", "admission_failure"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |_| Row::failure(vec![Value::Bool(true)], "host value differs from I64"),
            native_required: false,
        },
        Case {
            name: "wrong_arity",
            coverage: &["boundary_arity", "admission_failure"],
            source: "v2 (pub run (x:i64) i64 = (+ x 1))",
            row: |_| {
                Row::failure(
                    vec![],
                    "host argument or input count differs from interface",
                )
            },
            native_required: false,
        },
    ]
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalize(value: Result<Value, fluxr::Error>) -> Outcome {
    value.map_err(|error| error.to_string())
}

// Every field is derived from the retained observations. Nearest-rank p0 is
// the minimum; p100 is the maximum. These are empirical, not population bounds.
fn statistics(samples: &[f64], unit: &str) -> Json {
    assert!(!samples.is_empty());
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |tenths: usize| sorted[((sorted.len() * tenths).div_ceil(1000)).max(1) - 1];
    let all: Vec<_> = (0..=100).map(|p| percentile(p * 10)).collect();
    json!({"observation_unit":unit,"sample_count":samples.len(),"samples_ns":samples,
        "percentiles_p0_through_p100_ns":all,"p99_9_ns":percentile(999),
        "quantile_method":"nearest-rank; p0 is minimum",
        "expected_observations_above_p99_9":samples.len() as f64 / 1000.0})
}

fn same_result(case: &Case, row: &Row, actual: Result<Value, fluxr::Error>) {
    if let (Ok(Value::Record(value)), Some(Value::Record(original))) =
        (&actual, row.arguments.first())
    {
        if case.name == "record_read" {
            assert!(std::sync::Arc::ptr_eq(value, original));
        } else if case.name == "record_update" {
            assert!(!std::sync::Arc::ptr_eq(value, original));
        } else if case.name == "loop_record_update" {
            let Value::I64(iterations) = row.arguments[1] else {
                panic!("record loop fixture requires a signed iteration count");
            };
            assert_eq!(std::sync::Arc::ptr_eq(value, original), iterations == 0);
        }
    }
    if case.name == "list_copy"
        && let (Ok(Value::List(value)), Some(Value::List(original))) =
            (&actual, row.arguments.first())
    {
        assert!(!std::sync::Arc::ptr_eq(value, original));
    }
    assert_eq!(normalize(actual), row.expected, "{}", case.name);
}

struct Settings {
    samples: usize,
    rounds: usize,
    calls: usize,
    common_routes: bool,
}

fn setting(
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let value = std::env::var(name)
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("{name} is outside {min}..={max}").into());
    }
    Ok(value)
}

// Result disposal and result checking are outside individual-call timing. Batch
// timing includes disposal, with black_box preventing dead-code elimination.
fn measure(
    case: &Case,
    rows: &[Row],
    settings: &Settings,
    mut routes: Vec<(
        &str,
        &mut dyn FnMut(&Row) -> (Result<Value, fluxr::Error>, &'static str),
    )>,
) -> Json {
    let count = routes.len();
    let mut per_call: Vec<Vec<f64>> = (0..count)
        .map(|_| Vec::with_capacity(settings.samples))
        .collect();
    let mut batch: Vec<Vec<f64>> = (0..count)
        .map(|_| Vec::with_capacity(settings.rounds))
        .collect();
    let mut counts: Vec<BTreeMap<&str, usize>> = (0..count).map(|_| BTreeMap::new()).collect();
    for (_, call) in &mut routes {
        for i in 0..WARMUP {
            let row = &rows[i % rows.len()];
            same_result(case, row, call(row).0);
        }
    }
    // Rotate the first route on each input; all routes see exactly the same row.
    for i in 0..settings.samples {
        let row = &rows[i % rows.len()];
        for offset in 0..count {
            let k = (i + offset) % count;
            let start = Instant::now();
            let (result, route) = black_box(routes[k].1(black_box(row)));
            let elapsed = start.elapsed().as_nanos() as f64;
            per_call[k].push(elapsed);
            *counts[k].entry(route).or_default() += 1;
            same_result(case, row, result);
        }
    }
    for round in 0..settings.rounds {
        for offset in 0..count {
            let k = (round + offset) % count;
            let start = Instant::now();
            for i in 0..settings.calls {
                let _ = black_box(routes[k].1(black_box(&rows[i % rows.len()])));
            }
            batch[k].push(start.elapsed().as_nanos() as f64 / settings.calls as f64);
        }
    }
    let mut output = serde_json::Map::new();
    for (k, (name, _)) in routes.iter().enumerate() {
        output.insert((*name).to_owned(),json!({
            "individual_call":statistics(&per_call[k],"ns per invocation; timer included; result checking/disposal excluded"),
            "batch_mean":statistics(&batch[k],"ns per call averaged over a batch; result disposal included"),
            "timed_individual_route_counts":counts[k]}));
    }
    Json::Object(output)
}

fn lifecycle(case: &Case, row: &Row) -> Result<Json, Box<dyn std::error::Error>> {
    let mut compile = Vec::new();
    let mut decode = Vec::new();
    let mut prepare = Vec::new();
    let mut first = Vec::new();
    let mut cached = Vec::new();
    let mut destroy = Vec::new();
    #[cfg(not(fluxr_public_baseline))]
    let mut canonical_compile = Vec::new();
    for _ in 0..LIFECYCLE {
        let start = Instant::now();
        let module = Module::compile_compact(black_box(case.source))?;
        compile.push(start.elapsed().as_nanos() as f64);
        let bytes = module.encode();
        let start = Instant::now();
        let decoded = Module::decode(black_box(&bytes))?;
        decode.push(start.elapsed().as_nanos() as f64);
        assert_eq!(decoded.encode(), bytes);
        #[cfg(not(fluxr_public_baseline))]
        {
            let canonical = module.unified_text()?;
            let start = Instant::now();
            let canonical_module = SourceDocument::parse(black_box(&canonical))?.compile()?;
            canonical_compile.push(start.elapsed().as_nanos() as f64);
            assert_eq!(canonical_module.encode(), bytes);
        }
        let start = Instant::now();
        #[cfg(not(fluxr_public_baseline))]
        let call = module.prepare_call("run")?;
        #[cfg(fluxr_public_baseline)]
        black_box(module.prepare_native("run"));
        prepare.push(start.elapsed().as_nanos() as f64);
        // A new workspace is part of first-call cost; warm calls reuse capacity.
        let start = Instant::now();
        #[cfg(not(fluxr_public_baseline))]
        let result = call.run(&row.inputs, &row.arguments, row.limits);
        #[cfg(fluxr_public_baseline)]
        let result = module.run(row.invocation());
        first.push(start.elapsed().as_nanos() as f64);
        same_result(case, row, result);
        let start = Instant::now();
        #[cfg(not(fluxr_public_baseline))]
        black_box(module.prepare_call("run")?);
        #[cfg(fluxr_public_baseline)]
        black_box(module.prepare_native("run"));
        cached.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        drop(module);
        destroy.push(start.elapsed().as_nanos() as f64);
    }
    let mut result = json!({"compact_compile":statistics(&compile,"ns per source parse/check/compile"),
        "artifact_decode":statistics(&decode,"ns per artifact decode/validation"),
        "cold_prepare":statistics(&prepare,"ns per cold preparation; includes code publication when admitted"),
        "first_call":statistics(&first,"ns per first invocation with fresh workspace"),
        "cached_prepare":statistics(&cached,"ns per cached preparation"),
        "module_drop":statistics(&destroy,"ns per prepared module destruction")});
    #[cfg(not(fluxr_public_baseline))]
    {
        result["canonical_compile"] =
            statistics(&canonical_compile, "ns per FluxR 1 parse/check/compile");
    }
    #[cfg(fluxr_public_baseline)]
    {
        result["canonical_compile_unavailable"] =
            json!("public API has no canonical SourceDocument");
    }
    Ok(result)
}

#[cfg(not(fluxr_public_baseline))]
fn route_name(route: ExecutionRoute) -> &'static str {
    match route {
        ExecutionRoute::Portable => "Portable",
        ExecutionRoute::AheadOfTime => "AheadOfTime",
        ExecutionRoute::NativeScalar => "NativeScalar",
        ExecutionRoute::TypedJit => "TypedJit",
        ExecutionRoute::PrecompiledScan => "PrecompiledScan",
    }
}

fn run_case(case: &Case, settings: &Settings) -> Result<Json, Box<dyn std::error::Error>> {
    let module = Module::compile_compact(case.source)?;
    let bytes = module.encode();
    let decoded = Module::decode(&bytes)?;
    assert_eq!(decoded.encode(), bytes);
    #[cfg(not(fluxr_public_baseline))]
    let canonical = decoded.unified_text()?;
    #[cfg(not(fluxr_public_baseline))]
    let canonical_identity = (Some(digest(canonical.as_bytes())), Some(canonical.len()));
    #[cfg(fluxr_public_baseline)]
    let canonical_identity: (Option<String>, Option<usize>) = (None, None);
    #[cfg(not(fluxr_public_baseline))]
    let prepared = decoded.prepare_call("run")?;
    #[cfg(fluxr_public_baseline)]
    black_box(decoded.prepare_native("run"));
    let rows: Vec<_> = (0..VALIDATION_ROWS).map(case.row).collect();
    let identity = json!(rows.iter().map(Row::identity).collect::<Vec<_>>());
    let mut portable_ws = Workspace::default();
    let mut named_ws = Workspace::default();
    #[cfg(not(fluxr_public_baseline))]
    let mut prepared_ws = Workspace::default();
    let mut actual_routes = BTreeMap::new();
    for row in &rows {
        same_result(
            case,
            row,
            decoded.run_portable_with(row.invocation(), &mut portable_ws),
        );
        same_result(case, row, decoded.run_with(row.invocation(), &mut named_ws));
        #[cfg(not(fluxr_public_baseline))]
        {
            let (result, route) =
                prepared.run_with_route(&row.inputs, &row.arguments, row.limits, &mut prepared_ws);
            same_result(case, row, result);
            *actual_routes.entry(route_name(route)).or_insert(0usize) += 1;
            if case.native_required
                && cfg!(all(
                    feature = "typed-jit",
                    target_os = "linux",
                    target_env = "gnu",
                    target_arch = "x86_64"
                ))
            {
                assert_eq!(
                    route,
                    ExecutionRoute::TypedJit,
                    "{} must execute generated code",
                    case.name
                );
            }
        }
        #[cfg(fluxr_public_baseline)]
        {
            *actual_routes.entry("Unattributed").or_insert(0usize) += 1;
        }
    }
    let mut portable = |row: &Row| {
        (
            decoded.run_portable_with(row.invocation(), &mut portable_ws),
            "Portable",
        )
    };
    // Named API lacks per-call attribution. A separate prepared witness is
    // reported, but is never relabeled as instrumentation of named calls.
    let mut named = |row: &Row| {
        (
            decoded.run_with(row.invocation(), &mut named_ws),
            "Unattributed",
        )
    };
    #[cfg(not(fluxr_public_baseline))]
    let mut prepared_call = |row: &Row| {
        let (result, route) =
            prepared.run_with_route(&row.inputs, &row.arguments, row.limits, &mut prepared_ws);
        (result, route_name(route))
    };
    let mut routes: Vec<(
        &str,
        &mut dyn FnMut(&Row) -> (Result<Value, fluxr::Error>, &'static str),
    )> = vec![("portable", &mut portable), ("named", &mut named)];
    #[cfg(not(fluxr_public_baseline))]
    if !settings.common_routes {
        routes.push(("prepared", &mut prepared_call));
    }
    let timings = measure(case, &rows, settings, routes);
    // Source-owned fixture values remain unchanged across native, portable,
    // repeated success, trap, and copy-on-write outcomes.
    assert_eq!(
        identity,
        json!(rows.iter().map(Row::identity).collect::<Vec<_>>())
    );
    Ok(
        json!({"name":case.name,"coverage":case.coverage,"source_format":"compact-v2",
        "source_sha256":digest(case.source.as_bytes()),"input_limits_oracle_sha256":digest(&serde_json::to_vec(&identity)?),
        "canonical_source_sha256":canonical_identity.0,"canonical_source_bytes":canonical_identity.1,
        "artifact_sha256":digest(&bytes),"artifact_bytes":bytes.len(),
        "validation_rows":rows.len(),"validation_route_counts":actual_routes,
        "native_required_on_supported_candidate":case.native_required,
        "timings":timings,"lifecycle":lifecycle(case,&rows[0])?}),
    )
}

fn timer_calibration() -> Json {
    let mut overhead = Vec::with_capacity(4096);
    let mut injected = Vec::with_capacity(4096);
    let delay_ns = 200_000u128;
    for n in 0..4096 {
        let start = Instant::now();
        black_box(n);
        overhead.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        if n % 100 == 0 {
            while start.elapsed().as_nanos() < delay_ns {
                std::hint::spin_loop();
            }
        }
        black_box(n);
        injected.push(start.elapsed().as_nanos() as f64);
    }
    let means: Vec<_> = injected
        .chunks(64)
        .map(|x| x.iter().sum::<f64>() / x.len() as f64)
        .collect();
    json!({"empty_timer":statistics(&overhead,"ns per empty timer observation; not subtracted"),
        "injected_pause":statistics(&injected,"ns per calibration invocation; every hundredth contains a bounded spin pause"),
        "calibration_batch_means":statistics(&means,"mean ns per calibration invocation, reconstructed in groups of 64"),
        "injected_pause_ns":delay_ns,"injection_interval":100,"injected_indices":(0..4096).step_by(100).collect::<Vec<_>>(),
        "scope":"Timer sensitivity check; synthetic pauses are separate from workload observations and not scheduling or deadline qualification."})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--startup") {
        println!("fluxr-fragments-ready");
        return Ok(());
    }
    fluxr::telemetry::configure("off")?;
    let settings = Settings {
        samples: setting("FLUXR_FRAGMENT_SAMPLES", 10_000, 1024, 16_384)?,
        rounds: setting("FLUXR_FRAGMENT_ROUNDS", 31, 5, 101)?,
        calls: setting("FLUXR_FRAGMENT_BATCH_CALLS", 256, 32, 4096)?,
        common_routes: cfg!(fluxr_public_baseline)
            || match std::env::var("FLUXR_FRAGMENT_COMMON_ROUTES").as_deref() {
                Ok("1") => true,
                Ok("0") | Err(_) => false,
                _ => return Err("FLUXR_FRAGMENT_COMMON_ROUTES must be 0 or 1".into()),
            },
    };
    let started = Instant::now();
    let calibration = timer_calibration();
    let results = cases()
        .iter()
        .map(|case| run_case(case, &settings))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "{}",
        serde_json::to_string(&json!({"schema":"fluxr-fragment-performance-v1",
        "workload_catalog":"checked-fragments-v3",
        "probe_sha256":digest(include_bytes!("bench_fragments.rs")),
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "target_env":if cfg!(target_env="gnu") {"gnu"} else if cfg!(target_env="msvc") {"msvc"} else {"other"},
        "typed_jit_feature":cfg!(feature="typed-jit"),"debug_assertions":cfg!(debug_assertions),
        "public_baseline_adapter":cfg!(fluxr_public_baseline),"telemetry":"off",
        "numerical_profile":"checked integers; strict binary32; unchanged logical limits",
        "settings":{"individual_samples":settings.samples,"batch_rounds":settings.rounds,"calls_per_batch":settings.calls,
            "common_routes":settings.common_routes,"timed_routes":if settings.common_routes {vec!["portable","named"]} else {vec!["portable","named","prepared"]},
            "validation_rows":VALIDATION_ROWS,"warmup_calls_per_route":WARMUP,"lifecycle_samples":LIFECYCLE},
        "route_order":"rotate first route by individual input index or batch round",
        "row_order":"repeat deterministic 64-row corpus, same row for each route",
        "elapsed_ns":started.elapsed().as_nanos(),"calibration":calibration,"cases":results,
        "unmeasured":["native services and runtime registry/control/publication are separate integration gates",
            "process startup is collected by the capture tool, compiler process cost by the build runner",
            "GPU, native ML/audio/devices, concurrency, operating-system scheduling, live agents",
            "isolated generated machine-code kernel (these routes include checked host boundaries)",
            "Rust/.NET speed parity; this probe is a same-FluxR-API version comparison"],
        "claim_scope":"Finite empirical distributions, including all p0..p100 and p99.9. One build/run does not establish a population tail guarantee, statistical improvement, all-language coverage, or all-percentile nonregression."}))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_match_independent_results_and_preserve_host_values() {
        for case in cases() {
            let module = Module::compile_compact(case.source)
                .unwrap_or_else(|e| panic!("{}: {e}", case.name));
            let decoded = Module::decode(&module.encode()).unwrap();
            #[cfg(not(fluxr_public_baseline))]
            let prepared = decoded.prepare_call("run").unwrap();
            let mut vm = Workspace::default();
            #[cfg(not(fluxr_public_baseline))]
            let mut native = Workspace::default();
            for n in 0..VALIDATION_ROWS {
                let row = (case.row)(n);
                let before = row.identity();
                same_result(
                    &case,
                    &row,
                    decoded.run_portable_with(row.invocation(), &mut vm),
                );
                #[cfg(not(fluxr_public_baseline))]
                {
                    let (result, route) = prepared.run_with_route(
                        &row.inputs,
                        &row.arguments,
                        row.limits,
                        &mut native,
                    );
                    same_result(&case, &row, result);
                    if case.native_required
                        && cfg!(all(
                            feature = "typed-jit",
                            target_os = "linux",
                            target_env = "gnu",
                            target_arch = "x86_64"
                        ))
                    {
                        assert_eq!(route, ExecutionRoute::TypedJit, "{}", case.name);
                    }
                }
                assert_eq!(before, row.identity(), "{}", case.name);
            }
        }
    }

    #[test]
    fn percentiles_include_endpoints_and_known_rare_tail() {
        let mut samples = vec![10.0; 1000];
        samples[0] = 100_000.0;
        samples[500] = 100_000.0;
        let stats = statistics(&samples, "ns per synthetic invocation");
        assert_eq!(stats["percentiles_p0_through_p100_ns"][0], 10.0);
        assert_eq!(stats["percentiles_p0_through_p100_ns"][50], 10.0);
        assert_eq!(stats["percentiles_p0_through_p100_ns"][100], 100_000.0);
        assert_eq!(stats["p99_9_ns"], 100_000.0);
        let means: Vec<_> = samples
            .chunks(100)
            .map(|v| v.iter().sum::<f64>() / 100.0)
            .collect();
        assert!(means.iter().copied().fold(0.0, f64::max) < 100_000.0);
    }
}
