use super::{CELLS, FRAMES, FUEL, Replay};
use fluxr::structured::{Limits, Module, Value};
use std::hint::black_box;

const SOURCE: &str = include_str!("binary32_boundary.fluxr");
const I64_EDGE: i64 = (1_i64 << 62) + (1_i64 << 38);
const LAST_BELOW_SATURATION: i64 = i64::MAX - (1_i64 << 39) + 1;

// Signed integers round directly to binary32, including values one unit off a tie.
const FROM_INT: [(i64, u32); 15] = [
    (0, 0),
    (1, 0x3f80_0000),
    (-1, 0xbf80_0000),
    (16_777_215, 0x4b7f_ffff),
    (16_777_217, 0x4b80_0000),
    (16_777_219, 0x4b80_0002),
    (-16_777_217, 0xcb80_0000),
    (-16_777_219, 0xcb80_0002),
    (I64_EDGE - 1, 0x5e80_0000),
    (I64_EDGE, 0x5e80_0000),
    (I64_EDGE + 1, 0x5e80_0001),
    (-I64_EDGE, 0xde80_0000),
    (-I64_EDGE - 1, 0xde80_0001),
    (i64::MIN, 0xdf00_0000),
    (i64::MAX, 0x5f00_0000),
];

const TO_INT: [(u32, i64); 19] = [
    (0, 0),
    (0x8000_0000, 0),
    (1, 0),
    (0x8000_0001, 0),
    (0x3f7f_ffff, 0),
    (0xbf7f_ffff, 0),
    (0x3fff_ffff, 1),
    (0xbfff_ffff, -1),
    (0x5eff_ffff, LAST_BELOW_SATURATION),
    (0x5f00_0000, i64::MAX),
    (0x5f00_0001, i64::MAX),
    (0xdeff_ffff, -LAST_BELOW_SATURATION),
    (0xdf00_0000, i64::MIN),
    (0xdf00_0001, i64::MIN),
    (0x7f80_0000, i64::MAX),
    (0xff80_0000, i64::MIN),
    (0x7fc0_0031, 0),
    (0x7f80_0001, 0),
    (0xff80_0011, 0),
];

const NAN: u32 = 0x7fc0_0000;
const ROOT: [(u32, u32); 18] = [
    (0, 0),
    (0x8000_0000, 0x8000_0000),
    (1, 0x1a35_04f3),
    (2, 0x1a80_0000),
    (0x007f_ffff, 0x1fff_ffff),
    (0x0080_0000, 0x2000_0000),
    (0x8000_0001, NAN),
    (0x3f7f_ffff, 0x3f7f_ffff),
    (0x3f80_0001, 0x3f80_0000),
    (0x4000_0000, 0x3fb5_04f3),
    (0x4080_0000, 0x4000_0000),
    (0xbf80_0000, NAN),
    (0x7f7f_ffff, 0x5f7f_ffff),
    (0x7f80_0000, 0x7f80_0000),
    (0xff80_0000, NAN),
    (0x7fc0_0031, NAN),
    (0x7f80_0001, NAN),
    (0xff80_0011, NAN),
];

const PROJECT: [(i64, i64); 8] = [
    (i64::MIN, 0),
    (-1, 0),
    (0, 0),
    (1, 1),
    (24, 4),
    (25, 5),
    (1 << 48, 16_777_216),
    (i64::MAX, 3_037_000_448),
];

fn rust_sqrt(bits: u32) -> u32 {
    let input = f32::from_bits(black_box(bits));
    if input.is_nan() || input < 0.0 {
        NAN
    } else {
        input.sqrt().to_bits()
    }
}

fn check_export<Input: Copy, Output: Copy + std::fmt::Debug + PartialEq>(
    module: &Module,
    export: &str,
    instructions: u64,
    cases: &[(Input, Output)],
    input: impl Fn(Input) -> Value,
    output: impl Fn(Output) -> Value,
    rust: impl Fn(Input) -> Output,
    wrong_type: &str,
) -> usize {
    assert!(!module.prepare_native(export), "{export}");
    assert_eq!(
        module.emit_native_rust(export).unwrap_err().to_string(),
        format!("unsupported native AOT export {export}")
    );
    let mut replay = Replay::new(module.clone(), export, &[], &[]);
    // One argument cell, two frame/slot cells, one returned scalar cell.
    let exact = Limits {
        instructions,
        cells: 4,
        frames: 1,
    };
    let short = Limits {
        instructions: instructions - 1,
        ..exact
    };
    for &(argument, expected) in cases {
        assert_eq!(rust(argument), expected, "{export} {:?}", input(argument));
        let args = [input(argument)];
        replay.check(&args, short, Err(FUEL));
        replay.check(&args, Limits { cells: 3, ..exact }, Err(CELLS));
        replay.check(&args, Limits { frames: 0, ..exact }, Err(FRAMES));
        replay.check(&args, Limits { cells: 3, ..short }, Err(FUEL));
        replay.check(&args, Limits { cells: 2, ..short }, Err(CELLS));
        replay.check(&args, exact, Ok(output(expected)));
    }
    let stopped = Limits {
        instructions: 0,
        cells: 0,
        frames: 0,
    };
    replay.check(
        &[],
        stopped,
        Err("host argument or input count differs from interface"),
    );
    let malformed = [Value::Bool(false)];
    replay.check(&malformed, stopped, Err(CELLS));
    replay.check(
        &malformed,
        Limits {
            cells: 1,
            ..stopped
        },
        Err(wrong_type),
    );
    replay.count
}

pub(crate) fn check_binary32_boundary() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert_eq!(
        Module::compile_compact(&module.compact_text().unwrap())
            .unwrap()
            .encode(),
        module.encode()
    );
    // Local read, one or three unary operations, return.
    check_export(
        &module,
        "from_int",
        3,
        &FROM_INT,
        Value::I64,
        Value::F32,
        |x| (black_box(x) as f32).to_bits(),
        "host value differs from I64",
    ) + check_export(
        &module,
        "to_int",
        3,
        &TO_INT,
        Value::F32,
        Value::I64,
        |bits| f32::from_bits(black_box(bits)) as i64,
        "host value differs from F32",
    ) + check_export(
        &module,
        "root",
        3,
        &ROOT,
        Value::F32,
        Value::F32,
        rust_sqrt,
        "host value differs from F32",
    ) + check_export(
        &module,
        "project",
        5,
        &PROJECT,
        Value::I64,
        Value::I64,
        |x| f32::from_bits(rust_sqrt((black_box(x) as f32).to_bits())) as i64,
        "host value differs from I64",
    )
}
