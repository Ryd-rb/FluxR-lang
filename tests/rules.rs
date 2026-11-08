use fluxr::RuleProgram;

#[test]
fn text_typed_and_wire_formats_agree() {
    let text = "r1 2\ni0 i1 < -5 8 ?";
    let typed = "[1,2,[[0,0],[0,1],[7],[1,-5],[1,8],[11]]]";
    let rule = RuleProgram::compile_text(text).unwrap();
    assert_eq!(rule.text(), text);
    assert_eq!(rule.structured(), typed);
    assert_eq!(RuleProgram::compile_structured(typed).unwrap(), rule);
    let bytes = rule.encode();
    assert_eq!(
        bytes,
        b"FXD\0\x01\x02\x06\0\0\0\0\x01\x07\x01\xfb\xff\xff\xff\xff\xff\xff\xff\x01\x08\0\0\0\0\0\0\0\x0b"
    );
    let restored = RuleProgram::decode(&bytes).unwrap();
    assert_eq!(
        restored.replay("[[[2,3],-5],[[3,2],8],[[4,4],8]]").unwrap(),
        3
    );
    assert!(restored.replay("[[[2,3],8]]").is_err());
    assert!(restored.evaluate(&[0]).is_err());
    assert!(restored.evaluate(&[0, 1, 2]).is_err());
}

#[test]
fn rejects_invalid_shapes_and_portable_artifacts() {
    for source in [
        "r5 0 0",
        "r1 17 0",
        "r1 0 i0",
        "r1 0",
        "r1 1 i0 +",
        "r1 0 2 3",
        "r1 0 4 5 ?",
        "r1 1 i255",
        "r1 0 1 @",
    ] {
        assert!(RuleProgram::compile_text(source).is_err(), "{source}");
    }
    let stack = format!("r1 0 {}{}", "0 ".repeat(33), "+ ".repeat(32));
    assert!(RuleProgram::compile_text(&stack).is_err());
    for source in [
        "[1,1,[[0,-1]]]",
        "[1,1,[[0]]]",
        "[1,0,[[255]]]",
        "[1,0,[[1,3,4]]]",
    ] {
        assert!(RuleProgram::compile_structured(source).is_err(), "{source}");
    }
    let bytes = RuleProgram::compile_text("r1 1 i0 4 +").unwrap().encode();
    for length in 0..bytes.len() {
        assert!(RuleProgram::decode(&bytes[..length]).is_err());
    }
    let mut with_trailer = bytes;
    with_trailer.push(0);
    assert!(RuleProgram::decode(&with_trailer).is_err());
}

#[test]
fn arithmetic_is_checked_and_select_is_eager() {
    for source in [
        "r1 0 9223372036854775807 1 +",
        "r1 0 -9223372036854775808 1 -",
        "r1 0 9223372036854775807 2 *",
        "r1 0 -9223372036854775808 -1 /",
        "r1 0 -9223372036854775808 -1 %",
        "r1 0 4 0 /",
        "r1 0 0 7 0 / 3 ?",
    ] {
        assert!(
            RuleProgram::compile_text(source)
                .unwrap()
                .evaluate(&[])
                .is_err()
        );
    }
    let rule = RuleProgram::compile_text("r1 2 i0 i1 / i0 i1 % +").unwrap();
    assert_eq!(rule.evaluate(&[-5, 2]).unwrap(), -3);
}

#[test]
fn prepared_rules_preserve_results_and_errors() {
    let edges = [
        i64::MIN,
        i64::MIN + 1,
        -10,
        -1,
        0,
        1,
        9,
        i64::MAX - 1,
        i64::MAX,
    ];
    for operation in ["+", "-", "*", "/", "%", "<", "=", "m", "M"] {
        let rule = RuleProgram::compile_text(&format!("r1 2 i0 i1 {operation}")).unwrap();
        let prepared = rule.clone().prepare();
        assert_eq!(prepared.program().encode(), rule.encode());
        for left in edges {
            for right in edges {
                assert_eq!(
                    prepared.evaluate(&[left, right]),
                    rule.evaluate(&[left, right])
                );
            }
        }
        assert_eq!(prepared.evaluate(&[]), rule.evaluate(&[]));
    }
    let rule = RuleProgram::compile_text("r1 3 i0 i1 i2 / 13 ?").unwrap();
    let prepared = rule.clone().prepare();
    for inputs in [[0, 100, 2], [1, 100, 2], [0, 100, 0], [-1, i64::MIN, -1]] {
        assert_eq!(prepared.evaluate(&inputs), rule.evaluate(&inputs));
    }
}

#[test]
fn generated_rule_names_cannot_add_host_code() {
    let rule = RuleProgram::compile_text("r1 1 i0 2 /").unwrap();
    for name in ["", "main() {}", "λ", "test::call"] {
        assert!(rule.rust_function(name).is_err());
    }
    let source = rule.rust_function("division").unwrap();
    assert!(source.contains("fn fluxr_division(_i0: i64"));
    assert!(source.contains(".checked_div(v1)?"));
}

#[test]
fn version_two_adds_bitwise_and_without_changing_version_one() {
    assert!(RuleProgram::compile_text("r1 2 i0 i1 &").is_err());
    assert!(RuleProgram::compile_structured("[1,2,[[0,0],[0,1],[12]]]").is_err());
    let rule = RuleProgram::compile_text("r2 2 i0 i1 &").unwrap();
    assert_eq!(rule.text(), "r2 2\ni0 i1 &");
    assert_eq!(rule.structured(), "[2,2,[[0,0],[0,1],[12]]]");
    assert_eq!(rule.encode(), b"FXD\0\x02\x02\x03\0\0\0\0\x01\x0c");
    assert_eq!(RuleProgram::decode(&rule.encode()).unwrap(), rule);
    let prepared = rule.clone().prepare();
    for pair in [
        [0, 1],
        [1, 1],
        [11, 13],
        [i64::MIN, -1],
        [i64::MAX, i64::MIN],
    ] {
        let expected = pair[0] & pair[1];
        assert_eq!(rule.evaluate(&pair).unwrap(), expected);
        assert_eq!(prepared.evaluate(&pair).unwrap(), expected);
    }
}

#[test]
fn version_three_float_comparisons_are_ordered_and_portable() {
    let edges = [
        0xffc0_0001, // negative quiet NaN
        0xff80_0000, // negative infinity
        0xbf80_0000, // -1
        0x8000_0001, // negative subnormal
        0x8000_0000, // -0
        0,
        1,           // positive subnormal
        0x3f80_0000, // 1
        0x7f80_0000, // infinity
        0x7f80_0001, // signaling NaN
        0x7fc0_0001, // quiet NaN
    ];
    for (token, opcode) in [("f<", 13), ("f<=", 14)] {
        let rule = RuleProgram::compile_text(&format!("r3 2 i0 i1 {token}")).unwrap();
        let prepared = rule.clone().prepare();
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        assert!(prepared.uses_native_code());
        let mut wire = *b"FXD\0\x03\x02\x03\0\0\0\0\x01\0";
        wire[12] = opcode;
        assert_eq!(rule.encode(), wire);
        assert_eq!(RuleProgram::decode(&wire).unwrap(), rule);
        assert_eq!(
            RuleProgram::compile_structured(&format!("[3,2,[[0,0],[0,1],[{opcode}]]]")).unwrap(),
            rule
        );
        for version in [1, 2] {
            wire[4] = version;
            assert!(RuleProgram::decode(&wire).is_err());
            assert!(RuleProgram::compile_text(&format!("r{version} 2 i0 i1 {token}")).is_err());
            assert!(
                RuleProgram::compile_structured(&format!("[{version},2,[[0,0],[0,1],[{opcode}]]]"))
                    .is_err()
            );
        }
        for left in edges {
            for right in edges {
                let lhs = f32::from_bits(left);
                let rhs = f32::from_bits(right);
                let expected = i64::from(if opcode == 13 { lhs < rhs } else { lhs <= rhs });
                // Both zero- and sign-extended transport preserve the low bits.
                let inputs = [i64::from(left), i64::from(right.cast_signed())];
                assert_eq!(rule.evaluate(&inputs).unwrap(), expected);
                assert_eq!(prepared.evaluate(&inputs).unwrap(), expected);
            }
        }
    }
    assert!(RuleProgram::compile_text("r2 0 f32:0.03").is_err());
    let literal = RuleProgram::compile_text("r3 0 f32:-0.03").unwrap();
    assert_eq!(
        literal.evaluate(&[]).unwrap(),
        i64::from((-0.03_f32).to_bits())
    );
    assert_eq!(RuleProgram::compile_text(&literal.text()).unwrap(), literal);
    assert_eq!(
        RuleProgram::compile_structured(&literal.structured()).unwrap(),
        literal
    );
}

#[test]
fn version_three_guard_short_circuits_without_changing_checked_arithmetic() {
    let text = "r3 2 i0 guard 120 i1 /";
    let typed = "[3,2,[[0,0],[15],[1,120],[0,1],[5]]]";
    let rule = RuleProgram::compile_text(text).unwrap();
    assert_eq!(rule, RuleProgram::compile_structured(typed).unwrap());
    let mut wire = rule.encode();
    assert_eq!(RuleProgram::decode(&wire).unwrap(), rule);
    wire[4] = 2;
    assert!(RuleProgram::decode(&wire).is_err());
    assert!(RuleProgram::compile_text("r2 2 i0 guard i1").is_err());
    assert!(RuleProgram::compile_text("r3 0 guard 1").is_err());
    assert!(RuleProgram::compile_text("r3 0 0 guard").is_err());
    let prepared = rule.clone().prepare();
    #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
    assert!(prepared.uses_native_code());
    for pair in [[0, 0], [0, 3], [1, 0], [1, 3], [-1, 3]] {
        let actual = rule.evaluate(&pair);
        if pair[0] == 0 {
            assert_eq!(actual.as_ref().ok(), Some(&0));
        } else if pair[1] != 0 {
            assert_eq!(actual.as_ref().ok(), Some(&40));
        } else {
            assert!(actual.is_err());
        }
        assert_eq!(prepared.evaluate(&pair), actual);
    }
    assert!(
        rule.rust_array_function("guard")
            .unwrap()
            .contains("return Some(0)")
    );
}

#[test]
fn version_four_float_arithmetic_preserves_ieee_results_and_canonical_nan() {
    let values = [
        -f32::MAX,
        -f32::MIN_POSITIVE,
        -1.0,
        -0.0,
        0.0,
        1.0,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::MAX,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::from_bits(0xff80_0001),
        f32::from_bits(0x7fc0_0001),
    ];
    for (token, opcode, arithmetic) in [
        ("f+", 16, (|a, b| a + b) as fn(f32, f32) -> f32),
        ("f-", 17, |a, b| a - b),
        ("f*", 18, |a, b| a * b),
        ("f/", 19, |a, b| a / b),
    ] {
        let text = format!("r4 2 # inputs\ni0 i1 {token} # result");
        let rule = RuleProgram::compile_text(&text).unwrap();
        assert_eq!(RuleProgram::compile_text(&rule.text()).unwrap(), rule);
        assert_eq!(
            RuleProgram::compile_structured(&format!("[4,2,[[0,0],[0,1],[{opcode}]]]")).unwrap(),
            rule
        );
        let mut wire = *b"FXD\0\x04\x02\x03\0\0\0\0\x01\0";
        wire[12] = opcode;
        assert_eq!(rule.encode(), wire);
        assert_eq!(RuleProgram::decode(&wire).unwrap(), rule);
        wire[4] = 3;
        assert!(RuleProgram::decode(&wire).is_err());
        assert!(RuleProgram::compile_text(&format!("r3 2 i0 i1 {token}")).is_err());

        let prepared = rule.clone().prepare();
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        assert!(prepared.uses_native_code());
        for left in values {
            for right in values {
                let inputs = [i64::from(left.to_bits()), i64::from(right.to_bits())];
                let result = arithmetic(left, right);
                let bits = i64::from(if result.is_nan() {
                    0x7fc0_0000
                } else {
                    result.to_bits()
                });
                assert_eq!(rule.evaluate(&inputs).unwrap(), bits);
                assert_eq!(prepared.evaluate(&inputs).unwrap(), bits);
            }
        }
    }
    assert!(RuleProgram::compile_text("r3 0 1 # comment").is_err());
}
