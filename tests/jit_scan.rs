//! One-pass scan regressions. Values use an independent partial-order oracle;
//! malformed-value and low-resource outcomes are compared with portable replay.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use std::cmp::Ordering;

fn reference(bits: &[u32]) -> u64 {
    let Some(&first) = bits.first() else {
        return 0;
    };
    let mut selected = (0, f32::from_bits(first));
    for (index, &bits) in bits.iter().enumerate().skip(1) {
        let next = f32::from_bits(bits);
        if next.partial_cmp(&selected.1) == Some(Ordering::Less) {
            selected = (index, next);
        }
    }
    selected.0 as u64
}

fn list(bits: &[u32]) -> Value {
    Value::List(
        bits.iter()
            .copied()
            .map(Value::F32)
            .collect::<Vec<_>>()
            .into(),
    )
}

fn slices(values: &[Value], input: bool) -> (&[Value], &[Value]) {
    if input { (values, &[]) } else { (&[], values) }
}

fn module(input: bool) -> Module {
    Module::compile_compact(if input {
        "v2 (inputs samples:[f32]) (pub pick () u64 = (minidx $samples))"
    } else {
        "v2 (pub pick (samples:[f32]) u64 = (minidx samples))"
    })
    .unwrap()
}

fn compare(
    module: &Module,
    inputs: &[Value],
    arguments: &[Value],
    limits: Limits,
    workspace: &mut Workspace,
) -> (Result<Value, fluxr::Error>, ExecutionRoute) {
    let expected = module.run_portable_with(
        Invocation {
            export: "pick",
            inputs,
            arguments,
            limits,
        },
        &mut Workspace::default(),
    );
    let actual = module
        .prepare_call("pick")
        .unwrap()
        .run_with_route(inputs, arguments, limits, workspace);
    assert_eq!(actual.0, expected, "{limits:?}");
    actual
}

#[test]
fn validated_scan_preserves_nan_zero_tie_and_empty_rules_in_both_boundary_shapes() {
    let cases: &[&[u32]] = &[
        &[],
        &[1.0f32.to_bits()],
        &[
            0x7fc0_0041,
            (-1.0f32).to_bits(),
            f32::NEG_INFINITY.to_bits(),
        ],
        &[2.0f32.to_bits(), 0xffc0_0009, (-1.0f32).to_bits()],
        &[2.0f32.to_bits(), (-1.0f32).to_bits(), 0x7f80_0001],
        &[0x7fc0_0041, 0xffc0_0009, 0x7f80_0001],
        &[0, 0x8000_0000],
        &[0x8000_0000, 0],
        &[2.0f32.to_bits(), (-1.0f32).to_bits(), (-1.0f32).to_bits()],
        &[
            f32::INFINITY.to_bits(),
            1,
            0x8000_0001,
            f32::NEG_INFINITY.to_bits(),
        ],
    ];
    for input in [false, true] {
        let module = module(input);
        let mut workspace = Workspace::default();
        for &bits in cases {
            let values = [list(bits)];
            let (inputs, arguments) = slices(&values, input);
            let exact = Limits {
                instructions: bits.len() as u64 + 3,
                cells: bits.len() + if input { 3 } else { 4 },
                frames: 1,
            };
            for limits in [exact, Limits::default()] {
                let (value, route) = compare(&module, inputs, arguments, limits, &mut workspace);
                assert_eq!(value, Ok(Value::U64(reference(bits))));
                if cfg!(all(
                    feature = "typed-jit",
                    target_arch = "x86_64",
                    any(
                        target_os = "windows",
                        all(target_os = "linux", target_env = "gnu")
                    )
                )) {
                    assert_eq!(route, ExecutionRoute::PrecompiledScan);
                }
            }
            for instructions in 0..=exact.instructions + 1 {
                for cells in 0..=exact.cells + 1 {
                    for frames in 0..=1 {
                        let limits = Limits {
                            instructions,
                            cells,
                            frames,
                        };
                        let (value, route) =
                            compare(&module, inputs, arguments, limits, &mut workspace);
                        if instructions < exact.instructions || cells < exact.cells || frames == 0 {
                            assert!(value.is_err());
                            assert_eq!(route, ExecutionRoute::Portable);
                        } else {
                            assert_eq!(value, Ok(Value::U64(reference(bits))));
                        }
                    }
                }
            }
            // Wrong arity wins even when every resource limit is zero.
            let (value, route) = compare(
                &module,
                &values,
                &values,
                Limits {
                    instructions: 0,
                    cells: 0,
                    frames: 0,
                },
                &mut workspace,
            );
            assert_eq!(
                value.unwrap_err().to_string(),
                "host argument or input count differs from interface"
            );
            assert_eq!(route, ExecutionRoute::Portable);
            assert_eq!(values[0], list(bits));
        }
    }
}

#[test]
fn malformed_scan_tags_replay_with_cell_fuel_and_frame_error_precedence() {
    for input in [false, true] {
        let module = module(input);
        let mut workspace = Workspace::default();
        for bad_index in 0..3 {
            // A later bad tag must be checked even after an unbeatable valid
            // minimum; a future early-exit optimization must not skip it.
            let mut items = vec![
                Value::F32(f32::NEG_INFINITY.to_bits()),
                Value::F32(0),
                Value::F32(0x8000_0000),
            ];
            items[bad_index] = Value::Unit;
            let values = [Value::List(items.into())];
            let (inputs, arguments) = slices(&values, input);
            for instructions in 0..=7 {
                for cells in 0..=8 {
                    for frames in 0..=1 {
                        let (value, route) = compare(
                            &module,
                            inputs,
                            arguments,
                            Limits {
                                instructions,
                                cells,
                                frames,
                            },
                            &mut workspace,
                        );
                        assert!(value.is_err());
                        assert_eq!(route, ExecutionRoute::Portable);
                    }
                }
            }
        }
    }
}

// Decode finite binary32 values exactly into *normal* binary64 encodings,
// without a binary32 arithmetic/conversion instruction. The comparison oracle
// is binary64 partial ordering, independent of the implementation's u32 keys.
// Even the smallest binary32 subnormal is a normal binary64 value, so DAZ/FTZ
// cannot silently make this oracle equate nonzero binary32 values with zero.
fn finite_as_f64(bits: u32) -> f64 {
    let sign = u64::from(bits >> 31) << 63;
    let exponent = (bits >> 23) & 0xff;
    let fraction = bits & 0x7f_ffff;
    assert!(exponent != 0xff);
    let (exponent, fraction) = if exponent == 0 {
        if fraction == 0 {
            return f64::from_bits(sign);
        }
        let top = 31 - fraction.leading_zeros();
        (
            u64::from(top + 874),
            u64::from(fraction ^ (1 << top)) << (52 - top),
        )
    } else {
        (u64::from(exponent + 896), u64::from(fraction) << 29)
    };
    f64::from_bits(sign | (exponent << 52) | fraction)
}

#[test]
fn finite_bit_pairs_match_independent_exact_binary64_ordering() {
    let module = module(false);
    let prepared = module.prepare_call("pick").unwrap();
    let mut workspace = Workspace::default();
    let mut oracle_workspace = Workspace::default();
    let mut check = |left: u32, right: u32| {
        let expected = u64::from(
            finite_as_f64(right).partial_cmp(&finite_as_f64(left)) == Some(Ordering::Less),
        );
        // Exercise the single remainder, both lanes of a complete pair, and
        // that pair followed by a remainder. Both compared values are finite,
        // so a leading/trailing +infinity cannot change their relative order.
        for (bits, expected) in [
            (vec![left, right], expected),
            (vec![0x7f80_0000, left, right], expected + 1),
            (vec![0x7f80_0000, left, right, 0x7f80_0000], expected + 1),
        ] {
            let limits = Limits {
                instructions: bits.len() as u64 + 3,
                cells: bits.len() + 4,
                frames: 1,
            };
            let args = [list(&bits)];
            assert_eq!(
                prepared.run_with(&[], &args, limits, &mut workspace),
                Ok(Value::U64(expected)),
                "bits={bits:08x?}",
            );
            assert_eq!(
                module.run_portable_with(
                    Invocation {
                        export: "pick",
                        inputs: &[],
                        arguments: &args,
                        limits
                    },
                    &mut oracle_workspace,
                ),
                Ok(Value::U64(expected)),
            );
        }
    };
    let magnitudes = [
        0,
        1,
        2,
        0x007f_ffff,
        0x0080_0000,
        0x0080_0001,
        0x3f7f_ffff,
        0x3f80_0000,
        0x3f80_0001,
        0x7f7f_ffff,
    ];
    let anchors: Vec<_> = magnitudes
        .into_iter()
        .flat_map(|bits| [bits, bits | 0x8000_0000])
        .collect();
    for &left in &anchors {
        for &right in &anchors {
            check(left, right);
        }
    }
    let mut state = 0xa7c3_29d5u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        // Preserve both signs and all significands; map only exponent 255 to
        // finite exponent 254. NaNs/infinities have dedicated tests above.
        if state & 0x7f80_0000 == 0x7f80_0000 {
            state ^ 0x0080_0000
        } else {
            state
        }
    };
    for _ in 0..4096 {
        check(next(), next());
    }
}

#[cfg(target_arch = "x86_64")]
mod floating_environment {
    use super::*;
    use std::arch::asm;

    fn read_mxcsr() -> u32 {
        let mut value = 0u32;
        // SAFETY: x86-64 supplies SSE state; the live output slot is four bytes.
        unsafe {
            asm!("stmxcsr [{}]", in(reg) &mut value, options(nostack, preserves_flags));
        }
        value
    }

    unsafe fn write_mxcsr(value: u32) {
        // SAFETY: callers supply a previously read value or mask every changed
        // bit against FXSAVE's supported mask. No temporary mode unmasks traps.
        unsafe {
            asm!("ldmxcsr [{}]", in(reg) &value, options(nostack, preserves_flags));
        }
    }

    struct MxcsrGuard(u32);

    impl MxcsrGuard {
        fn masked_denorm_mode() -> (Self, bool) {
            #[repr(C, align(16))]
            struct FxState([u8; 512]);
            let mut state = FxState([0; 512]);
            // SAFETY: FXSAVE is supported on x86-64 and this 512-byte output is
            // 16-byte aligned. Saving state does not change the current MXCSR.
            unsafe {
                asm!("fxsave [{}]", in(reg) state.0.as_mut_ptr(), options(nostack, preserves_flags));
            }
            let reported = u32::from_le_bytes(state.0[28..32].try_into().unwrap());
            // The architectural fallback excludes optional DAZ. Do not write a
            // reserved DAZ bit on an implementation that does not advertise it.
            let supported = if reported == 0 { 0x0000_ffbf } else { reported };
            let saved = read_mxcsr();
            let has_daz = supported & 0x0040 != 0;
            let configured = (saved | 0x1f80 | 0x8000 | (supported & 0x0040)) & !0x003f;
            assert_eq!(configured & !supported, 0, "unsupported MXCSR bit");
            let guard = Self(saved);
            // SAFETY: all six exception classes are masked. Only supported
            // denormal controls are enabled, and sticky status flags are cleared.
            unsafe {
                write_mxcsr(configured);
            }
            (guard, has_daz)
        }
    }

    impl Drop for MxcsrGuard {
        fn drop(&mut self) {
            // SAFETY: restore the exact thread-local state saved before the test,
            // including during Rust unwinding. No other thread is modified.
            unsafe {
                write_mxcsr(self.0);
            }
        }
    }

    #[test]
    fn scan_preserves_mxcsr_and_ieee_subnormals_with_denormal_modes_enabled() {
        let module = module(false);
        let prepared = module.prepare_call("pick").unwrap();
        let cases = [
            (list(&[0x8000_0001, 0x8000_0002, 0]), 1),
            (list(&[2, 1, 0x0080_0000]), 1),
            (list(&[0x7f80_0001, 0xff80_0000]), 0),
            (list(&[0x3f80_0000, 0x7f80_0001, 1]), 2),
            (list(&[0, 0x8000_0000]), 0),
        ];
        let malformed = [Value::List(
            vec![Value::F32(0x7f80_0001), Value::Unit].into(),
        )];
        let mut workspace = Workspace::default();
        let mut portable_workspace = Workspace::default();
        let saved = read_mxcsr();
        let has_daz;
        {
            let (_guard, enabled) = MxcsrGuard::masked_denorm_mode();
            has_daz = enabled;
            let configured = read_mxcsr();
            assert_eq!(configured & 0x1f80, 0x1f80, "exceptions must stay masked");
            for (values, expected) in &cases {
                let args = std::slice::from_ref(values);
                let result = prepared.run_with(&[], args, Limits::default(), &mut workspace);
                assert_eq!(result, Ok(Value::U64(*expected)));
                assert_eq!(read_mxcsr(), configured, "prepared scan changed FP state");
                let result = module.run_portable_with(
                    Invocation {
                        export: "pick",
                        inputs: &[],
                        arguments: args,
                        limits: Limits::default(),
                    },
                    &mut portable_workspace,
                );
                assert_eq!(result, Ok(Value::U64(*expected)));
                assert_eq!(read_mxcsr(), configured, "portable scan changed FP state");
            }
            assert!(
                prepared
                    .run_with(&[], &malformed, Limits::default(), &mut workspace)
                    .is_err()
            );
            assert_eq!(
                read_mxcsr(),
                configured,
                "tag-error replay changed FP state"
            );
        }
        assert_eq!(read_mxcsr(), saved, "MXCSR was not restored");
        eprintln!("scan MXCSR coverage: FTZ=true DAZ={has_daz}, exceptions masked");
    }
}

#[test]
fn leading_signed_signaling_and_quiet_nans_still_validate_every_later_tag() {
    for input in [false, true] {
        let module = module(input);
        let mut workspace = Workspace::default();
        for nan in [0x7f80_0001, 0xff80_0001, 0x7fc0_0041, 0xffc0_0009] {
            let bits = [nan, 0xff80_0000, 0];
            let valid = [list(&bits)];
            let (inputs, arguments) = slices(&valid, input);
            let (value, _) = compare(
                &module,
                inputs,
                arguments,
                Limits::default(),
                &mut workspace,
            );
            assert_eq!(value, Ok(Value::U64(0)));
            for bad_index in [1, 2] {
                let mut items: Vec<_> = bits.into_iter().map(Value::F32).collect();
                items[bad_index] = Value::Unit;
                let malformed = [Value::List(items.into())];
                let (inputs, arguments) = slices(&malformed, input);
                for instructions in 0..=8 {
                    for cells in 0..=8 {
                        for frames in 0..=1 {
                            let (value, route) = compare(
                                &module,
                                inputs,
                                arguments,
                                Limits {
                                    instructions,
                                    cells,
                                    frames,
                                },
                                &mut workspace,
                            );
                            assert!(value.is_err(), "nan={nan:08x} bad_index={bad_index}");
                            assert_eq!(route, ExecutionRoute::Portable);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn paired_scan_retains_first_zero_and_tie_across_pair_and_remainder_boundaries() {
    let cases: &[(&[u32], u64)] = &[
        (&[0x4000_0000, 0, 0x8000_0000], 1),
        (&[0x4000_0000, 0x8000_0000, 0], 1),
        (&[0x4000_0000, 0x4000_0000, 0, 0x8000_0000], 2),
        (&[0x4000_0000, 0, 0x4000_0000, 0x8000_0000], 1),
        (&[0x4000_0000, 0x3f80_0000, 0x3f80_0000], 1),
        (&[0x4000_0000, 0x3f80_0000, 0x4000_0000, 0x3f80_0000], 1),
        (&[0x4000_0000, 0x3f80_0000, 0xff80_0001, 0xbf80_0000], 3),
        (
            &[
                0x4000_0000,
                0xff80_0001,
                0x7f80_0001,
                0,
                0x8000_0000,
                0x8000_0001,
                0x8000_0002,
            ],
            6,
        ),
        (
            &[0, 0x8000_0000, 0x7f80_0001, 0xff80_0001, 0, 0x8000_0000],
            0,
        ),
    ];
    for input in [false, true] {
        let module = module(input);
        let mut workspace = Workspace::default();
        for &(bits, expected) in cases {
            let values = [list(bits)];
            let (inputs, arguments) = slices(&values, input);
            let exact = Limits {
                instructions: bits.len() as u64 + 3,
                cells: bits.len() + if input { 3 } else { 4 },
                frames: 1,
            };
            for limits in [exact, Limits::default()] {
                let (value, _) = compare(&module, inputs, arguments, limits, &mut workspace);
                assert_eq!(value, Ok(Value::U64(expected)), "bits={bits:08x?}");
            }
        }
    }
}

#[test]
fn paired_scan_rejects_bad_tags_in_each_lane_and_in_odd_even_remainders() {
    for input in [false, true] {
        let module = module(input);
        let mut workspace = Workspace::default();
        for length in 2..=8 {
            let exact = Limits {
                instructions: length as u64 + 3,
                cells: length + if input { 3 } else { 4 },
                frames: 1,
            };
            for bad_index in 0..length {
                let mut items = vec![Value::F32(0); length];
                // An unbeatable early minimum must not skip any later tag.
                items[0] = Value::F32(0xff80_0000);
                items[bad_index] = Value::Unit;
                let values = [Value::List(items.into())];
                let (inputs, arguments) = slices(&values, input);
                for limits in [
                    exact,
                    Limits {
                        instructions: exact.instructions - 1,
                        ..exact
                    },
                    Limits {
                        cells: exact.cells - 1,
                        ..exact
                    },
                    Limits { frames: 0, ..exact },
                    Limits::default(),
                ] {
                    let (value, route) =
                        compare(&module, inputs, arguments, limits, &mut workspace);
                    assert!(value.is_err(), "length={length} bad_index={bad_index}");
                    assert_eq!(route, ExecutionRoute::Portable);
                }
            }
        }
    }
}
