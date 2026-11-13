//! Exact Rust controls and shared adapters for the cookbook tests and benchmark.
//! Reference files remain ordinary Rust kernels; adapters are counted separately.
#![allow(dead_code)]
#[cfg(not(fluxr_public_baseline))]
use fluxr::structured::SourceDocument;
use fluxr::structured::{Invocation, Limits, Module, Value};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::hint::black_box;

pub type Outcome = Result<Value, String>;
pub const ARITHMETIC: &str = "invalid or overflowing integer arithmetic";
pub enum KernelValue {
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    F32(u32),
    List(Vec<i64>),
    Point([i64; 2]),
    OptionalI64(Option<i64>),
}
pub type KernelOutcome = Result<KernelValue, &'static str>;
pub type Kernel = Box<dyn FnMut() -> KernelOutcome>;
pub fn kernel_outcome(value: KernelOutcome) -> Outcome {
    value
        .map(|value| match value {
            KernelValue::Unit => Value::Unit,
            KernelValue::Bool(x) => Value::Bool(x),
            KernelValue::I64(x) => Value::I64(x),
            KernelValue::U64(x) => Value::U64(x),
            KernelValue::F32(x) => Value::F32(x),
            KernelValue::List(xs) => list(&xs),
            KernelValue::Point(xs) => record(&xs),
            KernelValue::OptionalI64(x) => optional_value(x),
        })
        .map_err(str::to_owned)
}
fn kernel_integer(value: Option<i64>) -> KernelOutcome {
    value.map(KernelValue::I64).ok_or(ARITHMETIC)
}
fn kernel_unsigned(value: Option<u64>) -> KernelOutcome {
    value.map(KernelValue::U64).ok_or(ARITHMETIC)
}
#[derive(Clone, Debug)]
pub struct Row {
    pub inputs: Vec<Value>,
    pub arguments: Vec<Value>,
}
impl Row {
    pub fn new(arguments: Vec<Value>) -> Self {
        Self {
            inputs: vec![],
            arguments,
        }
    }
    pub fn invocation(&self, limits: Limits) -> Invocation<'_> {
        Invocation {
            export: "run",
            inputs: &self.inputs,
            arguments: &self.arguments,
            limits,
        }
    }
    pub fn identity(&self) -> Json {
        json!({"inputs":self.inputs,"arguments":self.arguments})
    }
}
pub struct Case {
    pub name: &'static str,
    pub topics: &'static [&'static str],
    pub source: &'static str,
    pub rust_source: &'static str,
    pub compact: &'static str,
    pub oracle: fn(&Row) -> Outcome,
    pub kernel: fn(&Row) -> Kernel,
    pub rows: Vec<Row>,
}
impl Case {
    pub fn compile(&self) -> Result<Module, fluxr::Error> {
        #[cfg(not(fluxr_public_baseline))]
        {
            SourceDocument::parse(self.source)?.compile()
        }
        #[cfg(fluxr_public_baseline)]
        {
            Module::compile_compact(self.compact)
        }
    }
    pub fn identity(&self) -> Json {
        json!({"name":self.name,"rows":self.rows.iter().map(|r|json!({"row":r.identity(),"expected":(self.oracle)(r)})).collect::<Vec<_>>(),"limits":Limits::default()})
    }
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn arithmetic_error() -> String {
    "invalid or overflowing integer arithmetic".into()
}
fn integer(value: Option<i64>) -> Outcome {
    value.map(Value::I64).ok_or_else(arithmetic_error)
}
fn unsigned(value: Option<u64>) -> Outcome {
    value.map(Value::U64).ok_or_else(arithmetic_error)
}
pub fn list(xs: &[i64]) -> Value {
    Value::List(
        xs.iter()
            .copied()
            .map(Value::I64)
            .collect::<Vec<_>>()
            .into(),
    )
}
pub fn record(xs: &[i64]) -> Value {
    Value::Record(
        xs.iter()
            .copied()
            .map(Value::I64)
            .collect::<Vec<_>>()
            .into(),
    )
}
fn as_i(value: &Value) -> i64 {
    match value {
        Value::I64(x) => *x,
        _ => panic!("oracle requires i64"),
    }
}
fn i(row: &Row, n: usize) -> i64 {
    as_i(&row.arguments[n])
}
fn u(row: &Row, n: usize) -> u64 {
    match row.arguments[n] {
        Value::U64(x) => x,
        _ => panic!("oracle requires u64"),
    }
}
fn b(row: &Row, n: usize) -> bool {
    match row.arguments[n] {
        Value::Bool(x) => x,
        _ => panic!("oracle requires bool"),
    }
}
fn f(row: &Row, n: usize) -> u32 {
    match row.arguments[n] {
        Value::F32(x) => x,
        _ => panic!("oracle requires f32 bits"),
    }
}
fn as_list(value: &Value) -> &[Value] {
    match value {
        Value::List(x) => x,
        _ => panic!("oracle requires list"),
    }
}
fn as_record(value: &Value) -> &[Value] {
    match value {
        Value::Record(x) => x,
        _ => panic!("oracle requires record"),
    }
}
fn as_ints(value: &Value) -> Vec<i64> {
    as_list(value).iter().map(as_i).collect()
}
fn ints(row: &Row, n: usize) -> Vec<i64> {
    as_ints(&row.arguments[n])
}
fn floats(row: &Row, n: usize) -> Vec<u32> {
    as_list(&row.arguments[n])
        .iter()
        .map(|v| match v {
            Value::F32(x) => *x,
            _ => panic!("float list"),
        })
        .collect()
}
fn fields(row: &Row, n: usize) -> &[Value] {
    as_record(&row.arguments[n])
}
fn variant(row: &Row, n: usize) -> (usize, Option<&Value>) {
    match &row.arguments[n] {
        Value::Variant(tag, payload) => (*tag, payload.as_deref()),
        _ => panic!("oracle requires enum"),
    }
}
fn optional_i(row: &Row, n: usize) -> Option<i64> {
    let (tag, payload) = variant(row, n);
    if tag == 0 {
        None
    } else {
        Some(as_i(payload.unwrap()))
    }
}
fn optional_value(value: Option<i64>) -> Value {
    match value {
        None => Value::Variant(0, None),
        Some(x) => Value::Variant(1, Some(Value::I64(x).into())),
    }
}

// Seeded bounded generators are deterministic across versions and platforms.
// Explicit boundary rows precede the pseudorandom interior values.
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
fn signed_rows(arity: usize) -> Vec<Row> {
    let edge = [
        i64::MIN,
        i64::MIN + 1,
        -16777217,
        -100,
        -1,
        0,
        1,
        100,
        16777217,
        i64::MAX - 1,
        i64::MAX,
    ];
    let mut rows = vec![];
    for k in 0..edge.len() {
        rows.push(Row::new(
            (0..arity)
                .map(|n| Value::I64(edge[(k + n * 3) % edge.len()]))
                .collect(),
        ));
    }
    if arity == 2 {
        rows.push(Row::new(vec![Value::I64(i64::MIN), Value::I64(-1)]));
        rows.push(Row::new(vec![Value::I64(i64::MIN), Value::I64(0)]));
    }
    let mut seed = 0x6421_77a1_005e_1eed;
    for _ in 0..64 {
        rows.push(Row::new(
            (0..arity)
                .map(|_| Value::I64((next(&mut seed) % 201) as i64 - 100))
                .collect(),
        ));
    }
    rows
}
fn unsigned_rows(arity: usize) -> Vec<Row> {
    let edge = [0, 1, 2, 17, 16777217, u64::MAX - 1, u64::MAX];
    let mut rows = vec![];
    for k in 0..edge.len() {
        rows.push(Row::new(
            (0..arity)
                .map(|n| Value::U64(edge[(k + n * 3) % edge.len()]))
                .collect(),
        ));
    }
    let mut seed = 0x32a1_8723_00ee_0191;
    for _ in 0..64 {
        rows.push(Row::new(
            (0..arity)
                .map(|_| Value::U64(next(&mut seed) % 1000))
                .collect(),
        ));
    }
    rows
}
fn list_rows() -> Vec<Row> {
    let mut rows = vec![
        vec![],
        vec![0],
        vec![1, 2, 3],
        vec![i64::MAX, 1],
        vec![i64::MIN, -1],
        vec![1, -1, 1, -1],
    ];
    let mut seed = 0x219d_6749_123e_07f1;
    for n in 0..64 {
        rows.push(
            (0..n % 19)
                .map(|_| (next(&mut seed) % 17) as i64 - 8)
                .collect(),
        );
    }
    rows.into_iter()
        .map(|xs| Row::new(vec![list(&xs)]))
        .collect()
}
fn record_rows() -> Vec<Row> {
    signed_rows(2)
        .into_iter()
        .map(|r| Row::new(vec![record(&[i(&r, 0), 7]), r.arguments[1].clone()]))
        .collect()
}
fn option_rows() -> Vec<Row> {
    signed_rows(2)
        .into_iter()
        .enumerate()
        .map(|(n, r)| {
            Row::new(vec![
                optional_value(if n % 3 == 0 { None } else { Some(i(&r, 0)) }),
                r.arguments[1].clone(),
            ])
        })
        .collect()
}
const FLOAT_BITS: [u32; 18] = [
    0, 0x80000000, 1, 0x80000001, 0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40800000,
    0x7f800000, 0xff800000, 0x7fc00001, 0xffc00001, 0x7f800001, 0xff800001, 0x5effffff, 0x5f000000,
    0xdf000000,
];
fn float_rows() -> Vec<Row> {
    let mut bits = FLOAT_BITS.to_vec();
    let mut seed = 0x3d82_24a9_07ab_1821;
    for _ in 0..64 {
        bits.push(next(&mut seed) as u32);
    }
    bits.into_iter()
        .map(|b| Row::new(vec![Value::F32(b)]))
        .collect()
}
fn float_list_rows() -> Vec<Row> {
    let mut rows = vec![
        vec![],
        vec![0x7fc00001, 0xff800000],
        vec![0, 0x80000000],
        vec![0x80000000, 0],
        vec![0x3f800000, 0xbf800000, 0xbf800000],
    ];
    for n in 0..64 {
        rows.push(
            (0..n % 23)
                .map(|i| FLOAT_BITS[(i * 7 + n) % FLOAT_BITS.len()])
                .collect(),
        );
    }
    rows.into_iter()
        .map(|bits| {
            Row::new(vec![Value::List(
                bits.into_iter().map(Value::F32).collect::<Vec<_>>().into(),
            )])
        })
        .collect()
}
#[path = "boolean_logic.rs"]
pub mod boolean_logic;
#[path = "checked_affine.rs"]
pub mod checked_affine;
#[path = "clamp.rs"]
pub mod clamp;
#[path = "enum_list.rs"]
pub mod enum_list;
#[path = "enum_map.rs"]
pub mod enum_map;
#[path = "enum_option.rs"]
pub mod enum_option;
#[path = "factorial.rs"]
pub mod factorial;
#[path = "fibonacci.rs"]
pub mod fibonacci;
#[path = "five_arguments.rs"]
pub mod five_arguments;
#[path = "float_absolute.rs"]
pub mod float_absolute;
#[path = "float_affine.rs"]
pub mod float_affine;
#[path = "float_minimum.rs"]
pub mod float_minimum;
#[path = "float_sqrt.rs"]
pub mod float_sqrt;
#[path = "float_to_int.rs"]
pub mod float_to_int;
#[path = "gcd.rs"]
pub mod gcd;
#[path = "guarded_division.rs"]
pub mod guarded_division;
#[path = "helper_calls.rs"]
pub mod helper_calls;
#[path = "input_adjust.rs"]
pub mod input_adjust;
#[path = "int_to_float.rs"]
pub mod int_to_float;
#[path = "list_contains.rs"]
pub mod list_contains;
#[path = "list_count.rs"]
pub mod list_count;
#[path = "list_index.rs"]
pub mod list_index;
#[path = "list_last.rs"]
pub mod list_last;
#[path = "list_push_pop.rs"]
pub mod list_push_pop;
#[path = "list_reverse.rs"]
pub mod list_reverse;
#[path = "list_sum.rs"]
pub mod list_sum;
#[path = "loop_exits.rs"]
pub mod loop_exits;
#[path = "method_shift.rs"]
pub mod method_shift;
#[path = "nested_list.rs"]
pub mod nested_list;
#[path = "nested_record.rs"]
pub mod nested_record;
#[path = "power.rs"]
pub mod power;
#[path = "record_point.rs"]
pub mod record_point;
#[path = "record_update.rs"]
pub mod record_update;
#[path = "recursive_sum.rs"]
pub mod recursive_sum;
#[path = "tagged_payload.rs"]
pub mod tagged_payload;
#[path = "triangular.rs"]
pub mod triangular;
#[path = "uint_to_float.rs"]
pub mod uint_to_float;
#[path = "uninitialized_order.rs"]
pub mod uninitialized_order;
#[path = "unit.rs"]
pub mod unit;
#[path = "unsigned_affine.rs"]
pub mod unsigned_affine;
#[path = "unsigned_division.rs"]
pub mod unsigned_division;
pub fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "unit",
            topics: &["unit", "zero_arguments"],
            source: include_str!("unit.fluxr"),
            rust_source: include_str!("unit.rs"),
            compact: include_str!("unit.compact"),
            oracle: |_| {
                unit::run();
                Ok(Value::Unit)
            },
            kernel: |_| {
                Box::new(move || {
                    unit::run();
                    Ok(KernelValue::Unit)
                })
            },
            rows: vec![Row::new(vec![])],
        },
        Case {
            name: "checked_affine",
            topics: &["i64", "checked_arithmetic", "overflow"],
            source: include_str!("checked_affine.fluxr"),
            rust_source: include_str!("checked_affine.rs"),
            compact: include_str!("checked_affine.compact"),
            oracle: |r| integer(checked_affine::run(i(r, 0), i(r, 1), i(r, 2))),
            kernel: |r| {
                let (a, b, c) = (i(r, 0), i(r, 1), i(r, 2));
                Box::new(move || {
                    kernel_integer(checked_affine::run(
                        black_box(a),
                        black_box(b),
                        black_box(c),
                    ))
                })
            },
            rows: signed_rows(3),
        },
        Case {
            name: "unsigned_affine",
            topics: &["u64", "checked_arithmetic", "overflow"],
            source: include_str!("unsigned_affine.fluxr"),
            rust_source: include_str!("unsigned_affine.rs"),
            compact: include_str!("unsigned_affine.compact"),
            oracle: |r| unsigned(unsigned_affine::run(u(r, 0), u(r, 1), u(r, 2))),
            kernel: |r| {
                let (a, b, c) = (u(r, 0), u(r, 1), u(r, 2));
                Box::new(move || {
                    kernel_unsigned(unsigned_affine::run(
                        black_box(a),
                        black_box(b),
                        black_box(c),
                    ))
                })
            },
            rows: unsigned_rows(3),
        },
        Case {
            name: "clamp",
            topics: &["if", "comparisons", "early_return"],
            source: include_str!("clamp.fluxr"),
            rust_source: include_str!("clamp.rs"),
            compact: include_str!("clamp.compact"),
            oracle: |r| Ok(Value::I64(clamp::run(i(r, 0), i(r, 1), i(r, 2)))),
            kernel: |r| {
                let (a, b, c) = (i(r, 0), i(r, 1), i(r, 2));
                Box::new(move || {
                    Ok(KernelValue::I64(clamp::run(
                        black_box(a),
                        black_box(b),
                        black_box(c),
                    )))
                })
            },
            rows: signed_rows(1)
                .into_iter()
                .map(|mut r| {
                    r.arguments.extend([Value::I64(-5), Value::I64(5)]);
                    r
                })
                .collect(),
        },
        Case {
            name: "guarded_division",
            topics: &["bool", "short_circuit", "division", "traps"],
            source: include_str!("guarded_division.fluxr"),
            rust_source: include_str!("guarded_division.rs"),
            compact: include_str!("guarded_division.compact"),
            oracle: |r| {
                guarded_division::run(i(r, 0), i(r, 1))
                    .map(Value::Bool)
                    .ok_or_else(arithmetic_error)
            },
            kernel: |r| {
                let (a, b) = (i(r, 0), i(r, 1));
                Box::new(move || {
                    guarded_division::run(black_box(a), black_box(b))
                        .map(KernelValue::Bool)
                        .ok_or(ARITHMETIC)
                })
            },
            rows: signed_rows(2),
        },
        Case {
            name: "gcd",
            topics: &["u64", "euclidean_algorithm", "while", "rem"],
            source: include_str!("gcd.fluxr"),
            rust_source: include_str!("gcd.rs"),
            compact: include_str!("gcd.compact"),
            oracle: |r| Ok(Value::U64(gcd::run(u(r, 0), u(r, 1)))),
            kernel: |r| {
                let (a, b) = (u(r, 0), u(r, 1));
                Box::new(move || Ok(KernelValue::U64(gcd::run(black_box(a), black_box(b)))))
            },
            rows: unsigned_rows(2),
        },
        Case {
            name: "factorial",
            topics: &["u64", "while", "checked_mul"],
            source: include_str!("factorial.fluxr"),
            rust_source: include_str!("factorial.rs"),
            compact: include_str!("factorial.compact"),
            oracle: |r| unsigned(factorial::run(u(r, 0))),
            kernel: |r| {
                let n = u(r, 0);
                Box::new(move || kernel_unsigned(factorial::run(black_box(n))))
            },
            rows: (0..=32).map(|n| Row::new(vec![Value::U64(n)])).collect(),
        },
        Case {
            name: "fibonacci",
            topics: &["i64", "recurrence", "locals", "overflow"],
            source: include_str!("fibonacci.fluxr"),
            rust_source: include_str!("fibonacci.rs"),
            compact: include_str!("fibonacci.compact"),
            oracle: |r| integer(fibonacci::run(i(r, 0))),
            kernel: |r| {
                let n = i(r, 0);
                Box::new(move || kernel_integer(fibonacci::run(black_box(n))))
            },
            rows: (-2..=94).map(|n| Row::new(vec![Value::I64(n)])).collect(),
        },
        Case {
            name: "power",
            topics: &["i64", "bounded_exponent", "checked_mul"],
            source: include_str!("power.fluxr"),
            rust_source: include_str!("power.rs"),
            compact: include_str!("power.compact"),
            oracle: |r| integer(power::run(i(r, 0), u(r, 1))),
            kernel: |r| {
                let (a, b) = (i(r, 0), u(r, 1));
                Box::new(move || kernel_integer(power::run(black_box(a), black_box(b))))
            },
            rows: (-3..=3)
                .flat_map(|x| (0..=64).map(move |n| Row::new(vec![Value::I64(x), Value::U64(n)])))
                .collect(),
        },
        Case {
            name: "triangular",
            topics: &["i64", "while", "compact_steps"],
            source: include_str!("triangular.fluxr"),
            rust_source: include_str!("triangular.rs"),
            compact: include_str!("triangular.compact"),
            oracle: |r| integer(triangular::run(i(r, 0))),
            kernel: |r| {
                let n = i(r, 0);
                Box::new(move || kernel_integer(triangular::run(black_box(n))))
            },
            rows: (-3..=64).map(|n| Row::new(vec![Value::I64(n)])).collect(),
        },
        Case {
            name: "loop_exits",
            topics: &["break", "continue", "while", "control_order"],
            source: include_str!("loop_exits.fluxr"),
            rust_source: include_str!("loop_exits.rs"),
            compact: include_str!("loop_exits.compact"),
            oracle: |r| integer(loop_exits::run(i(r, 0))),
            kernel: |r| {
                let n = i(r, 0);
                Box::new(move || kernel_integer(loop_exits::run(black_box(n))))
            },
            rows: (-3..=64).map(|n| Row::new(vec![Value::I64(n)])).collect(),
        },
        Case {
            name: "helper_calls",
            topics: &["private_function", "calls", "checked_arithmetic"],
            source: include_str!("helper_calls.fluxr"),
            rust_source: include_str!("helper_calls.rs"),
            compact: include_str!("helper_calls.compact"),
            oracle: |r| integer(helper_calls::run(i(r, 0), i(r, 1))),
            kernel: |r| {
                let (a, b) = (i(r, 0), i(r, 1));
                Box::new(move || kernel_integer(helper_calls::run(black_box(a), black_box(b))))
            },
            rows: signed_rows(2),
        },
        Case {
            name: "recursive_sum",
            topics: &["recursion", "frames", "base_case"],
            source: include_str!("recursive_sum.fluxr"),
            rust_source: include_str!("recursive_sum.rs"),
            compact: include_str!("recursive_sum.compact"),
            oracle: |r| integer(recursive_sum::run(i(r, 0))),
            kernel: |r| {
                let n = i(r, 0);
                Box::new(move || kernel_integer(recursive_sum::run(black_box(n))))
            },
            rows: (-3..=24).map(|n| Row::new(vec![Value::I64(n)])).collect(),
        },
        Case {
            name: "list_sum",
            topics: &["list", "copy_on_write", "checked_fold"],
            source: include_str!("list_sum.fluxr"),
            rust_source: include_str!("list_sum.rs"),
            compact: include_str!("list_sum.compact"),
            oracle: |r| integer(list_sum::run(&ints(r, 0))),
            kernel: |r| {
                let xs = ints(r, 0);
                Box::new(move || kernel_integer(list_sum::run(black_box(&xs))))
            },
            rows: list_rows(),
        },
        Case {
            name: "list_count",
            topics: &["list", "empty", "pop", "copy_on_write"],
            source: include_str!("list_count.fluxr"),
            rust_source: include_str!("list_count.rs"),
            compact: include_str!("list_count.compact"),
            oracle: |r| integer(list_count::run(&ints(r, 0))),
            kernel: |r| {
                let xs = ints(r, 0);
                Box::new(move || kernel_integer(list_count::run(black_box(&xs))))
            },
            rows: list_rows(),
        },
        Case {
            name: "list_reverse",
            topics: &["list", "allocation", "push", "pop", "ordering"],
            source: include_str!("list_reverse.fluxr"),
            rust_source: include_str!("list_reverse.rs"),
            compact: include_str!("list_reverse.compact"),
            oracle: |r| Ok(list(&list_reverse::run(&ints(r, 0)))),
            kernel: |r| {
                let xs = ints(r, 0);
                Box::new(move || Ok(KernelValue::List(list_reverse::run(black_box(&xs)))))
            },
            rows: list_rows(),
        },
        Case {
            name: "list_contains",
            topics: &["list", "search", "early_return", "bool"],
            source: include_str!("list_contains.fluxr"),
            rust_source: include_str!("list_contains.rs"),
            compact: include_str!("list_contains.compact"),
            oracle: |r| Ok(Value::Bool(list_contains::run(&ints(r, 0), i(r, 1)))),
            kernel: |r| {
                let xs = ints(r, 0);
                let x = i(r, 1);
                Box::new(move || {
                    Ok(KernelValue::Bool(list_contains::run(
                        black_box(&xs),
                        black_box(x),
                    )))
                })
            },
            rows: list_rows()
                .into_iter()
                .enumerate()
                .map(|(n, mut r)| {
                    r.arguments.push(Value::I64(n as i64 % 9 - 4));
                    r
                })
                .collect(),
        },
        Case {
            name: "list_last",
            topics: &["list", "guard", "short_circuit"],
            source: include_str!("list_last.fluxr"),
            rust_source: include_str!("list_last.rs"),
            compact: include_str!("list_last.compact"),
            oracle: |r| Ok(Value::I64(list_last::run(&ints(r, 0), i(r, 1)))),
            kernel: |r| {
                let xs = ints(r, 0);
                let x = i(r, 1);
                Box::new(move || {
                    Ok(KernelValue::I64(list_last::run(
                        black_box(&xs),
                        black_box(x),
                    )))
                })
            },
            rows: list_rows()
                .into_iter()
                .map(|mut r| {
                    r.arguments.push(Value::I64(-99));
                    r
                })
                .collect(),
        },
        Case {
            name: "list_push_pop",
            topics: &["list", "push", "pop", "copy_on_write", "snapshot"],
            source: include_str!("list_push_pop.fluxr"),
            rust_source: include_str!("list_push_pop.rs"),
            compact: include_str!("list_push_pop.compact"),
            oracle: |r| Ok(list(&list_push_pop::run(&ints(r, 0), i(r, 1)))),
            kernel: |r| {
                let xs = ints(r, 0);
                let x = i(r, 1);
                Box::new(move || {
                    Ok(KernelValue::List(list_push_pop::run(
                        black_box(&xs),
                        black_box(x),
                    )))
                })
            },
            rows: list_rows()
                .into_iter()
                .enumerate()
                .map(|(n, mut r)| {
                    r.arguments.push(Value::I64(n as i64));
                    r
                })
                .collect(),
        },
        Case {
            name: "list_index",
            topics: &["list", "bounds", "unsigned_index"],
            source: include_str!("list_index.fluxr"),
            rust_source: include_str!("list_index.rs"),
            compact: include_str!("list_index.compact"),
            oracle: |r| {
                list_index::run(&ints(r, 0), u(r, 1))
                    .map(Value::I64)
                    .ok_or_else(|| "list index out of bounds".into())
            },
            kernel: |r| {
                let xs = ints(r, 0);
                let x = u(r, 1);
                Box::new(move || {
                    list_index::run(black_box(&xs), black_box(x))
                        .map(KernelValue::I64)
                        .ok_or("list index out of bounds")
                })
            },
            rows: list_rows()
                .into_iter()
                .enumerate()
                .map(|(n, mut r)| {
                    r.arguments.push(Value::U64(n as u64 % 20));
                    r
                })
                .collect(),
        },
        Case {
            name: "record_point",
            topics: &["record", "construction", "checked_add"],
            source: include_str!("record_point.fluxr"),
            rust_source: include_str!("record_point.rs"),
            compact: include_str!("record_point.compact"),
            oracle: |r| {
                record_point::run(i(r, 0))
                    .map(|p| record(&[p.x, p.y]))
                    .ok_or_else(arithmetic_error)
            },
            kernel: |r| {
                let x = i(r, 0);
                Box::new(move || {
                    record_point::run(black_box(x))
                        .map(|p| KernelValue::Point([p.x, p.y]))
                        .ok_or(ARITHMETIC)
                })
            },
            rows: signed_rows(1),
        },
        Case {
            name: "record_update",
            topics: &["record", "field", "copy_on_write", "checked_add"],
            source: include_str!("record_update.fluxr"),
            rust_source: include_str!("record_update.rs"),
            compact: include_str!("record_update.compact"),
            oracle: |r| {
                let p = fields(r, 0);
                record_update::run(
                    record_update::Point {
                        x: as_i(&p[0]),
                        y: as_i(&p[1]),
                    },
                    i(r, 1),
                )
                .map(|p| record(&[p.x, p.y]))
                .ok_or_else(arithmetic_error)
            },
            kernel: |r| {
                let p = fields(r, 0);
                let (x, y, d) = (as_i(&p[0]), as_i(&p[1]), i(r, 1));
                Box::new(move || {
                    record_update::run(black_box(record_update::Point { x, y }), black_box(d))
                        .map(|p| KernelValue::Point([p.x, p.y]))
                        .ok_or(ARITHMETIC)
                })
            },
            rows: record_rows(),
        },
        Case {
            name: "nested_record",
            topics: &["record", "nesting", "projections"],
            source: include_str!("nested_record.fluxr"),
            rust_source: include_str!("nested_record.rs"),
            compact: include_str!("nested_record.compact"),
            oracle: |r| {
                let p = as_record(&fields(r, 0)[0]);
                integer(nested_record::run(nested_record::Envelope {
                    point: nested_record::Point {
                        x: as_i(&p[0]),
                        y: as_i(&p[1]),
                    },
                }))
            },
            kernel: |r| {
                let p = as_record(&fields(r, 0)[0]);
                let (x, y) = (as_i(&p[0]), as_i(&p[1]));
                Box::new(move || {
                    kernel_integer(nested_record::run(black_box(nested_record::Envelope {
                        point: nested_record::Point { x, y },
                    })))
                })
            },
            rows: signed_rows(2)
                .into_iter()
                .map(|r| {
                    Row::new(vec![Value::Record(
                        vec![record(&[i(&r, 0), i(&r, 1)])].into(),
                    )])
                })
                .collect(),
        },
        Case {
            name: "method_shift",
            topics: &["record", "nominal_method", "checked_add"],
            source: include_str!("method_shift.fluxr"),
            rust_source: include_str!("method_shift.rs"),
            compact: include_str!("method_shift.compact"),
            oracle: |r| {
                integer(method_shift::run(
                    method_shift::Point {
                        x: as_i(&fields(r, 0)[0]),
                    },
                    i(r, 1),
                ))
            },
            kernel: |r| {
                let (x, d) = (as_i(&fields(r, 0)[0]), i(r, 1));
                Box::new(move || {
                    kernel_integer(method_shift::run(
                        black_box(method_shift::Point { x }),
                        black_box(d),
                    ))
                })
            },
            rows: signed_rows(2)
                .into_iter()
                .map(|r| Row::new(vec![record(&[i(&r, 0)]), r.arguments[1].clone()]))
                .collect(),
        },
        Case {
            name: "enum_option",
            topics: &["enum", "optional_payload", "guard"],
            source: include_str!("enum_option.fluxr"),
            rust_source: include_str!("enum_option.rs"),
            compact: include_str!("enum_option.compact"),
            oracle: |r| Ok(Value::I64(enum_option::run(optional_i(r, 0), i(r, 1)))),
            kernel: |r| {
                let (x, d) = (optional_i(r, 0), i(r, 1));
                Box::new(move || {
                    Ok(KernelValue::I64(enum_option::run(
                        black_box(x),
                        black_box(d),
                    )))
                })
            },
            rows: option_rows(),
        },
        Case {
            name: "enum_map",
            topics: &["enum", "construct_payload", "checked_map"],
            source: include_str!("enum_map.fluxr"),
            rust_source: include_str!("enum_map.rs"),
            compact: include_str!("enum_map.compact"),
            oracle: |r| {
                enum_map::run(optional_i(r, 0), i(r, 1))
                    .map(optional_value)
                    .ok_or_else(arithmetic_error)
            },
            kernel: |r| {
                let (x, d) = (optional_i(r, 0), i(r, 1));
                Box::new(move || {
                    enum_map::run(black_box(x), black_box(d))
                        .map(KernelValue::OptionalI64)
                        .ok_or(ARITHMETIC)
                })
            },
            rows: option_rows(),
        },
        Case {
            name: "float_affine",
            topics: &["f32", "strict_binary32", "arithmetic", "nan"],
            source: include_str!("float_affine.fluxr"),
            rust_source: include_str!("float_affine.rs"),
            compact: include_str!("float_affine.compact"),
            oracle: |r| Ok(Value::F32(float_affine::run(f(r, 0)))),
            kernel: |r| {
                let x = f(r, 0);
                Box::new(move || Ok(KernelValue::F32(float_affine::run(black_box(x)))))
            },
            rows: float_rows(),
        },
        Case {
            name: "float_sqrt",
            topics: &["f32", "sqrt", "canonical_nan", "signed_zero"],
            source: include_str!("float_sqrt.fluxr"),
            rust_source: include_str!("float_sqrt.rs"),
            compact: include_str!("float_sqrt.compact"),
            oracle: |r| Ok(Value::F32(float_sqrt::run(f(r, 0)))),
            kernel: |r| {
                let x = f(r, 0);
                Box::new(move || Ok(KernelValue::F32(float_sqrt::run(black_box(x)))))
            },
            rows: float_rows(),
        },
        Case {
            name: "float_to_int",
            topics: &["f32", "i64", "saturating_conversion", "nan"],
            source: include_str!("float_to_int.fluxr"),
            rust_source: include_str!("float_to_int.rs"),
            compact: include_str!("float_to_int.compact"),
            oracle: |r| Ok(Value::I64(float_to_int::run(f(r, 0)))),
            kernel: |r| {
                let x = f(r, 0);
                Box::new(move || Ok(KernelValue::I64(float_to_int::run(black_box(x)))))
            },
            rows: float_rows(),
        },
        Case {
            name: "int_to_float",
            topics: &["i64", "f32", "direct_rounding"],
            source: include_str!("int_to_float.fluxr"),
            rust_source: include_str!("int_to_float.rs"),
            compact: include_str!("int_to_float.compact"),
            oracle: |r| Ok(Value::F32(int_to_float::run(i(r, 0)))),
            kernel: |r| {
                let x = i(r, 0);
                Box::new(move || Ok(KernelValue::F32(int_to_float::run(black_box(x)))))
            },
            rows: signed_rows(1),
        },
        Case {
            name: "uint_to_float",
            topics: &["u64", "f32", "direct_rounding"],
            source: include_str!("uint_to_float.fluxr"),
            rust_source: include_str!("uint_to_float.rs"),
            compact: include_str!("uint_to_float.compact"),
            oracle: |r| Ok(Value::F32(uint_to_float::run(u(r, 0)))),
            kernel: |r| {
                let x = u(r, 0);
                Box::new(move || Ok(KernelValue::F32(uint_to_float::run(black_box(x)))))
            },
            rows: unsigned_rows(1),
        },
        Case {
            name: "float_minimum",
            topics: &["f32", "list", "minidx", "tie", "nan", "signed_zero"],
            source: include_str!("float_minimum.fluxr"),
            rust_source: include_str!("float_minimum.rs"),
            compact: include_str!("float_minimum.compact"),
            oracle: |r| Ok(Value::U64(float_minimum::run(&floats(r, 0)))),
            kernel: |r| {
                let xs = floats(r, 0);
                Box::new(move || Ok(KernelValue::U64(float_minimum::run(black_box(&xs)))))
            },
            rows: float_list_rows(),
        },
        Case {
            name: "input_adjust",
            topics: &["inputs", "snapshots", "bool", "checked_arithmetic"],
            source: include_str!("input_adjust.fluxr"),
            rust_source: include_str!("input_adjust.rs"),
            compact: include_str!("input_adjust.compact"),
            oracle: |r| {
                let enabled = match r.inputs[0] {
                    Value::Bool(v) => v,
                    _ => unreachable!(),
                };
                integer(input_adjust::run(enabled, as_i(&r.inputs[1]), i(r, 0)))
            },
            kernel: |r| {
                let enabled = match r.inputs[0] {
                    Value::Bool(v) => v,
                    _ => unreachable!(),
                };
                let (base, delta) = (as_i(&r.inputs[1]), i(r, 0));
                Box::new(move || {
                    kernel_integer(input_adjust::run(
                        black_box(enabled),
                        black_box(base),
                        black_box(delta),
                    ))
                })
            },
            rows: signed_rows(2)
                .into_iter()
                .enumerate()
                .map(|(n, r)| Row {
                    arguments: vec![r.arguments[1].clone()],
                    inputs: vec![Value::Bool(n % 2 == 0), r.arguments[0].clone()],
                })
                .collect(),
        },
        Case {
            name: "five_arguments",
            topics: &["i64", "five_arguments", "generic_boundary"],
            source: include_str!("five_arguments.fluxr"),
            rust_source: include_str!("five_arguments.rs"),
            compact: include_str!("five_arguments.compact"),
            oracle: |r| {
                integer(five_arguments::run(
                    i(r, 0),
                    i(r, 1),
                    i(r, 2),
                    i(r, 3),
                    i(r, 4),
                ))
            },
            kernel: |r| {
                let (a, b, c, d, e) = (i(r, 0), i(r, 1), i(r, 2), i(r, 3), i(r, 4));
                Box::new(move || {
                    kernel_integer(five_arguments::run(
                        black_box(a),
                        black_box(b),
                        black_box(c),
                        black_box(d),
                        black_box(e),
                    ))
                })
            },
            rows: signed_rows(5),
        },
        Case {
            name: "uninitialized_order",
            topics: &["trap", "uninitialized", "first_error"],
            source: include_str!("uninitialized_order.fluxr"),
            rust_source: include_str!("uninitialized_order.rs"),
            compact: include_str!("uninitialized_order.compact"),
            oracle: |r| {
                uninitialized_order::run(None, i(r, 0))
                    .map(Value::I64)
                    .map_err(str::to_owned)
            },
            kernel: |r| {
                let d = i(r, 0);
                Box::new(move || {
                    uninitialized_order::run(black_box(None), black_box(d)).map(KernelValue::I64)
                })
            },
            rows: signed_rows(1),
        },
        Case {
            name: "tagged_payload",
            topics: &["enum", "exact_tag", "payload", "trap"],
            source: include_str!("tagged_payload.fluxr"),
            rust_source: include_str!("tagged_payload.rs"),
            compact: include_str!("tagged_payload.compact"),
            oracle: |r| {
                let (tag, payload) = variant(r, 0);
                let value = match tag {
                    0 => tagged_payload::Choice::Empty,
                    1 => tagged_payload::Choice::Number(as_i(payload.unwrap())),
                    _ => tagged_payload::Choice::Other(as_i(payload.unwrap())),
                };
                tagged_payload::run(value)
                    .map(Value::I64)
                    .ok_or_else(|| "enum payload does not match variant".into())
            },
            kernel: |r| {
                let (tag, payload) = variant(r, 0);
                let x = payload.map(as_i).unwrap_or(0);
                Box::new(move || {
                    let value = match tag {
                        0 => tagged_payload::Choice::Empty,
                        1 => tagged_payload::Choice::Number(x),
                        _ => tagged_payload::Choice::Other(x),
                    };
                    tagged_payload::run(black_box(value))
                        .map(KernelValue::I64)
                        .ok_or("enum payload does not match variant")
                })
            },
            rows: signed_rows(1)
                .into_iter()
                .enumerate()
                .map(|(n, r)| {
                    Row::new(vec![if n % 3 == 0 {
                        Value::Variant(0, None)
                    } else {
                        Value::Variant(n % 3, Some(r.arguments[0].clone().into()))
                    }])
                })
                .collect(),
        },
        Case {
            name: "unsigned_division",
            topics: &["u64", "checked_div", "division_by_zero"],
            source: include_str!("unsigned_division.fluxr"),
            rust_source: include_str!("unsigned_division.rs"),
            compact: include_str!("unsigned_division.compact"),
            oracle: |r| unsigned(unsigned_division::run(u(r, 0), u(r, 1))),
            kernel: |r| {
                let (a, b) = (u(r, 0), u(r, 1));
                Box::new(move || {
                    kernel_unsigned(unsigned_division::run(black_box(a), black_box(b)))
                })
            },
            rows: unsigned_rows(2),
        },
        Case {
            name: "enum_list",
            topics: &["enum", "list", "short_circuit", "payload"],
            source: include_str!("enum_list.fluxr"),
            rust_source: include_str!("enum_list.rs"),
            compact: include_str!("enum_list.compact"),
            oracle: |r| {
                let (tag, payload) = variant(r, 0);
                let values = payload.map(as_ints);
                Ok(Value::Bool(enum_list::run(if tag == 0 {
                    None
                } else {
                    Some(values.as_deref().unwrap())
                })))
            },
            kernel: |r| {
                let (tag, payload) = variant(r, 0);
                let xs = payload.map(as_ints);
                Box::new(move || {
                    Ok(KernelValue::Bool(enum_list::run(black_box(if tag == 0 {
                        None
                    } else {
                        Some(xs.as_deref().unwrap())
                    }))))
                })
            },
            rows: std::iter::once(Row::new(vec![Value::Variant(0, None)]))
                .chain(list_rows().into_iter().map(|r| {
                    Row::new(vec![Value::Variant(1, Some(r.arguments[0].clone().into()))])
                }))
                .collect(),
        },
        Case {
            name: "nested_list",
            topics: &["list", "nesting", "last", "ownership"],
            source: include_str!("nested_list.fluxr"),
            rust_source: include_str!("nested_list.rs"),
            compact: include_str!("nested_list.compact"),
            oracle: |r| {
                let rows = as_list(&r.arguments[0])
                    .iter()
                    .map(as_ints)
                    .collect::<Vec<_>>();
                nested_list::run(&rows)
                    .map(Value::I64)
                    .ok_or_else(|| "last from empty list".into())
            },
            kernel: |r| {
                let rows = as_list(&r.arguments[0])
                    .iter()
                    .map(as_ints)
                    .collect::<Vec<_>>();
                Box::new(move || {
                    nested_list::run(black_box(&rows))
                        .map(KernelValue::I64)
                        .ok_or("last from empty list")
                })
            },
            rows: std::iter::once(Row::new(vec![Value::List(vec![].into())]))
                .chain(list_rows().into_iter().map(|r| {
                    Row::new(vec![Value::List(
                        vec![list(&[1]), r.arguments[0].clone()].into(),
                    )])
                }))
                .collect(),
        },
        Case {
            name: "float_absolute",
            topics: &["f32", "abs", "raw_bits", "nan_payload"],
            source: include_str!("float_absolute.fluxr"),
            rust_source: include_str!("float_absolute.rs"),
            compact: include_str!("float_absolute.compact"),
            oracle: |r| Ok(Value::F32(float_absolute::run(f(r, 0)))),
            kernel: |r| {
                let x = f(r, 0);
                Box::new(move || Ok(KernelValue::F32(float_absolute::run(black_box(x)))))
            },
            rows: float_rows(),
        },
        Case {
            name: "boolean_logic",
            topics: &["bool", "not", "and", "or", "comparison"],
            source: include_str!("boolean_logic.fluxr"),
            rust_source: include_str!("boolean_logic.rs"),
            compact: include_str!("boolean_logic.compact"),
            oracle: |r| Ok(Value::Bool(boolean_logic::run(b(r, 0), b(r, 1), i(r, 2)))),
            kernel: |r| {
                let (a, b, x) = (b(r, 0), b(r, 1), i(r, 2));
                Box::new(move || {
                    Ok(KernelValue::Bool(boolean_logic::run(
                        black_box(a),
                        black_box(b),
                        black_box(x),
                    )))
                })
            },
            rows: (0..4)
                .flat_map(|flags| {
                    signed_rows(1).into_iter().map(move |r| {
                        Row::new(vec![
                            Value::Bool(flags & 1 != 0),
                            Value::Bool(flags & 2 != 0),
                            r.arguments[0].clone(),
                        ])
                    })
                })
                .collect(),
        },
    ]
}
