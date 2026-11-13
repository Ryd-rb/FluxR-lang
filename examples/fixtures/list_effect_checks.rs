use super::{CELLS, FRAMES, FUEL, Replay, SCAN, typed_effect_spec::reference_min};
use fluxr::structured::{Binding, Limits, Module, Type, Value};

const TAG: &str = "host value differs from F32";

pub(super) fn list(values: impl IntoIterator<Item = Value>) -> Value {
    Value::List(values.into_iter().collect::<Vec<_>>().into())
}

pub(crate) fn check_list_effects() -> usize {
    let module = Module::compile_compact(SCAN).unwrap();
    assert_eq!(
        module.prepare_native("pick"),
        cfg!(all(
            feature = "typed-jit",
            any(
                target_os = "windows",
                all(target_os = "linux", target_env = "gnu")
            ),
            target_arch = "x86_64"
        ))
    );
    let mut scan = Replay::new(module, "pick", &[], &[]);
    let patterns = [
        0xff80_0000, // -infinity
        0xbf80_0000, // -1
        0x8000_0000, // -0
        0,
        0x3f80_0000, // 1
        0x7f80_0000, // infinity
        0x7fc0_0031, // quiet NaN
        0x7f80_0001, // signaling NaN
        0xff80_0040, // negative signaling NaN
    ];
    for len in 0..=3 {
        for mut code in 0..patterns.len().pow(len) {
            let bits: Vec<_> = (0..len)
                .map(|_| {
                    let value = patterns[code % patterns.len()];
                    code /= patterns.len();
                    value
                })
                .collect();
            let argument = [list(bits.iter().copied().map(Value::F32))];
            // Load, scan dispatch, len dynamic fuel, return. Boundary visits
            // the wrapper and elements, then two frame cells and one result.
            let exact = Limits {
                instructions: u64::from(len) + 3,
                cells: len as usize + 4,
                frames: 1,
            };
            scan.check(&argument, exact, Ok(Value::U64(reference_min(&bits))));
            scan.check(
                &argument,
                Limits {
                    instructions: 1,
                    ..exact
                },
                Err(FUEL),
            );
            scan.check(
                &argument,
                Limits {
                    instructions: exact.instructions - 1,
                    cells: exact.cells - 1,
                    ..exact
                },
                Err(FUEL),
            );
            scan.check(
                &argument,
                Limits {
                    cells: exact.cells - 1,
                    ..exact
                },
                Err(CELLS),
            );
            scan.check(&argument, Limits { frames: 0, ..exact }, Err(FRAMES));
        }
    }

    for len in [16, 256] {
        let mut bits = vec![1.0_f32.to_bits(); len];
        bits[1] = 0xff80_0040;
        bits[len - 2] = 0;
        bits[len - 1] = 0x8000_0000;
        let exact = Limits {
            instructions: len as u64 + 3,
            cells: len + 4,
            frames: 1,
        };
        for first in [bits[0], 0x7f80_0001, 0x7fc0_0031] {
            bits[0] = first;
            let argument = [list(bits.iter().copied().map(Value::F32))];
            scan.check(&argument, exact, Ok(Value::U64(reference_min(&bits))));
            scan.check(
                &argument,
                Limits {
                    instructions: exact.instructions - 2,
                    ..exact
                },
                Err(FUEL),
            );
            scan.check(
                &argument,
                Limits {
                    cells: exact.cells - 1,
                    ..exact
                },
                Err(CELLS),
            );
        }
    }

    let limits = Limits {
        instructions: 0,
        frames: 0,
        ..Limits::default()
    };
    for bad in [Value::I64(0), list([Value::F32(0)])] {
        for position in 0..3 {
            let mut values = [Value::F32(0), Value::F32(0), Value::F32(0)];
            values[position] = bad.clone();
            let argument = [list(values)];
            for cells in 0..=4 {
                scan.check(
                    &argument,
                    Limits { cells, ..limits },
                    Err(if cells <= position + 1 { CELLS } else { TAG }),
                );
            }
        }
    }
    scan.check(&[Value::F32(0)], Limits { cells: 0, ..limits }, Err(CELLS));
    scan.check(
        &[Value::F32(0)],
        Limits { cells: 1, ..limits },
        Err("host value differs from List(F32)"),
    );
    scan.check(
        &[],
        Limits { cells: 0, ..limits },
        Err("host argument or input count differs from interface"),
    );

    // Host inputs precede export arguments. A valid input can consume all
    // cells before reaching a malformed argument.
    let host_source = "v2 (inputs samples:[f32])
        (pub pick (extra:[f32]) u64 () (ret (minidx (input samples))))";
    let module = Module::compile_compact(host_source).unwrap();
    let binding = [Binding {
        name: "samples".into(),
        ty: Type::List(Box::new(Type::F32)),
    }];
    let valid = [list([Value::F32(0)])];
    let invalid = [list([Value::Unit])];
    let mut host = Replay::new(module.clone(), "pick", &valid, &binding);
    host.check(&invalid, Limits { cells: 3, ..limits }, Err(CELLS));
    host.check(&invalid, Limits { cells: 4, ..limits }, Err(TAG));
    let mut bad_host = Replay::new(module, "pick", &invalid, &binding);
    bad_host.check(&valid, Limits { cells: 1, ..limits }, Err(CELLS));
    bad_host.check(&valid, Limits { cells: 2, ..limits }, Err(TAG));

    let module = Module::compile_compact(
        "v2 (inputs samples:[f32]) (pub pick () u64 () (ret (minidx (input samples))))",
    )
    .unwrap();
    assert_eq!(
        module.prepare_native("pick"),
        cfg!(all(
            feature = "typed-jit",
            any(
                target_os = "windows",
                all(target_os = "linux", target_env = "gnu")
            ),
            target_arch = "x86_64"
        ))
    );
    let mut single_input = Replay::new(module.clone(), "pick", &valid, &binding);
    let exact = Limits {
        instructions: 4,
        cells: 4,
        frames: 1,
    };
    single_input.check(&[], exact, Ok(Value::U64(0)));
    single_input.check(&[], Limits { cells: 3, ..exact }, Err(CELLS));
    single_input.check(&[], Limits { frames: 0, ..exact }, Err(FRAMES));
    let mut bad_single_input = Replay::new(module, "pick", &invalid, &binding);
    bad_single_input.check(&[], Limits { cells: 1, ..limits }, Err(CELLS));
    bad_single_input.check(&[], Limits { cells: 2, ..limits }, Err(TAG));

    // An empty scan returns zero and can therefore feed a later index trap.
    // The nonempty case shows that a first NaN still costs the full scan.
    let mut index = Replay::new(
        Module::compile_compact(
            "v2 (pub first (values:[f32]) f32 () (ret (at values (minidx values))))",
        )
        .unwrap(),
        "first",
        &[],
        &[],
    );
    index.check(
        &[list([])],
        Limits {
            instructions: 3,
            ..Limits::default()
        },
        Err(FUEL),
    );
    index.check(
        &[list([])],
        Limits {
            instructions: 4,
            ..Limits::default()
        },
        Err("list index out of bounds"),
    );
    let first_nan = [list([Value::F32(0x7fc0_0031), Value::F32(0)])];
    index.check(
        &first_nan,
        Limits {
            instructions: 4,
            ..Limits::default()
        },
        Err(FUEL),
    );
    index.check(
        &first_nan,
        Limits {
            instructions: 7,
            ..Limits::default()
        },
        Ok(Value::F32(0x7fc0_0031)),
    );

    scan.count
        + host.count
        + bad_host.count
        + single_input.count
        + bad_single_input.count
        + index.count
}
