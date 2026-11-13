//! Independent witnesses for WK1. The scalar/loop oracles below are fixed Rust
//! arithmetic and observations, not calls to the WK1 expander. The collection
//! differential uses the previously released compact reader and a hand-written
//! loop with distinct local names. Mock process routes live in a separate test.
use fluxr::structured::wordcode::{Definition, Dictionary, ExpansionLimits, ScalarType, Template};
use fluxr::structured::{Binary, Invocation, Limits, Module, Value, Workspace};
use std::sync::Arc;

fn core(source: &str) -> Module {
    Module::compile_wordcode(source, &Dictionary::core(), ExpansionLimits::default()).unwrap()
}

fn portable(module: &Module, arguments: &[Value], limits: Limits) -> Result<Value, String> {
    module
        .run_portable_with(
            Invocation {
                export: "calc",
                inputs: &[],
                arguments,
                limits,
            },
            &mut Workspace::default(),
        )
        .map_err(|error| error.to_string())
}

#[test]
fn counted_control_flow_matches_independent_odd_sum_oracle() {
    // The loop takes a snapshot of n, decrements privately before its body,
    // continues past even values, and stops before adding the requested value.
    let module = core(
        "wk1 core v2 pub calc 2 n int stop int int \
         do 2 x int total int 4 set x 0 set total 0 \
         repeat n 4 next x if eq x stop 1 break 0 \
         if eq rem x 2 0 1 continue 0 bump total x ret total",
    );
    for n in -3..18 {
        for stop in 0..21 {
            let mut expected = 0;
            for x in 1..=n {
                if x == stop {
                    break;
                }
                if x % 2 != 0 {
                    expected += x;
                }
            }
            let values = [Value::I64(n), Value::I64(stop)];
            assert_eq!(
                portable(&module, &values, Limits::default()),
                Ok(Value::I64(expected)),
                "n={n}, stop={stop}"
            );
        }
    }
}

#[test]
fn walk_rereads_aliased_step_and_preserves_nonprogress_and_overflow() {
    let module = core("wk1 core v2 pub calc 2 x int end int int do 0 2 walk x end x ret x");
    // `x` as the step doubles the current value; it is not a frozen original
    // increment. These answers deliberately distinguish the two contracts.
    for (initial, end, expected) in [(1, 17, 32), (3, 20, 24), (7, 7, 7), (0, 0, 0)] {
        assert_eq!(
            portable(
                &module,
                &[Value::I64(initial), Value::I64(end)],
                Limits::default()
            ),
            Ok(Value::I64(expected))
        );
    }
    assert_eq!(
        portable(
            &module,
            &[Value::I64(0), Value::I64(1)],
            Limits {
                instructions: 50,
                ..Limits::default()
            }
        ),
        Err("module instruction budget exhausted".into())
    );
    assert_eq!(
        portable(
            &module,
            &[Value::I64(i64::MAX / 2 + 1), Value::I64(i64::MAX)],
            Limits::default()
        ),
        Err("invalid or overflowing integer arithmetic".into())
    );
}

#[test]
fn frozen_admission_fuel_and_first_trap_table_is_identical_after_preparation() {
    let module = core("wk1 core v2 pub calc 1 x int int ret up x");
    let prepared = module.prepare_call("calc").unwrap();
    // Independent fixed boundaries: one argument cell, two frame cells, four
    // instructions, then one result cell. Overflow happens at instruction three.
    let rows = [
        (Value::I64(5), 4, 4, 1, Ok(Value::I64(6))),
        (
            Value::I64(5),
            3,
            4,
            1,
            Err("module instruction budget exhausted"),
        ),
        (
            Value::I64(i64::MAX),
            3,
            3,
            1,
            Err("invalid or overflowing integer arithmetic"),
        ),
        (
            Value::I64(i64::MAX),
            2,
            3,
            1,
            Err("module instruction budget exhausted"),
        ),
        (Value::I64(5), 4, 3, 1, Err("module cell budget exhausted")),
        (Value::I64(5), 4, 0, 0, Err("module cell budget exhausted")),
        (Value::I64(5), 4, 1, 0, Err("module frame limit exceeded")),
        (
            Value::Bool(true),
            4,
            0,
            0,
            Err("module cell budget exhausted"),
        ),
        (
            Value::Bool(true),
            4,
            1,
            0,
            Err("host value differs from I64"),
        ),
    ];
    for (value, instructions, cells, frames, expected) in rows {
        let values = [value];
        let limits = Limits {
            instructions,
            cells,
            frames,
        };
        let expected = expected.map_err(str::to_owned);
        assert_eq!(
            portable(&module, &values, limits),
            expected,
            "portable {limits:?}"
        );
        assert_eq!(
            prepared
                .run(&[], &values, limits)
                .map_err(|error| error.to_string()),
            expected,
            "prepared {limits:?}"
        );
    }
}

#[test]
fn compound_arithmetic_cannot_move_a_later_uninitialized_read_before_overflow() {
    let module = core("wk1 core v2 pub calc 1 x int int do 1 missing int 1 ret sum x 1 missing");
    assert_eq!(
        portable(&module, &[Value::I64(i64::MAX)], Limits::default()),
        Err("invalid or overflowing integer arithmetic".into())
    );
    assert_eq!(
        portable(&module, &[Value::I64(0)], Limits::default()),
        Err("read of uninitialized local".into())
    );
}

#[test]
fn float_compound_keeps_rounding_signed_zero_and_canonical_nan() {
    let module = core("wk1 core v2 pub calc 3 x float y float z float float ret blend x y z");
    let prepared = module.prepare_call("calc").unwrap();
    // (1+2^-23)*(1-2^-23)-1 rounds to zero with separate binary32
    // multiplication/addition. A fused operation would produce -2^-46.
    for (bits, expected) in [
        ([0x3f80_0001, 0x3f7f_fffe, 0xbf80_0000], 0),
        ([0x8000_0000, 0x3f80_0000, 0x8000_0000], 0x8000_0000),
        ([0x7fc1_2345, 0x3f80_0000, 0], 0x7fc0_0000),
    ] {
        let arguments = bits.map(Value::F32);
        assert_eq!(
            portable(&module, &arguments, Limits::default()),
            Ok(Value::F32(expected))
        );
        assert_eq!(
            prepared.run(&[], &arguments, Limits::default()).unwrap(),
            Value::F32(expected)
        );
    }
}

#[test]
fn repeated_list_updates_preserve_cow_and_match_released_compact_budget_oracle() {
    let module = core(
        "wk1 core v2 pub calc 2 n int values list int list int \
         do 2 copy list int x int 4 set copy values set x 0 \
         repeat n 2 next x push copy square x ret copy",
    );
    let ordinary = Module::compile_compact(
        "v2 (pub calc (n:i64 values:[i64]) [i64] (copy:[i64] x:i64 remaining:i64) \
         (= copy values) (= x 0) (= remaining n) \
         (while (< 0 remaining) ((= remaining (- remaining 1)) (= x (+ x 1)) \
         (push copy (* x x)))) (ret copy))",
    )
    .unwrap();
    let original = Arc::new(vec![Value::I64(9), Value::I64(10)]);
    for n in [-1, 0, 4] {
        let arguments = [Value::I64(n), Value::List(Arc::clone(&original))];
        let result = portable(&module, &arguments, Limits::default()).unwrap();
        let Value::List(result) = result else {
            panic!("list result required")
        };
        let expected: Vec<_> = [9, 10]
            .into_iter()
            .chain((1..=n).map(|x| x * x))
            .map(Value::I64)
            .collect();
        assert_eq!(*result, expected);
        assert_eq!(Arc::ptr_eq(&result, &original), n <= 0);
        assert_eq!(*original, [Value::I64(9), Value::I64(10)]);
        for instructions in (0..120).step_by(3) {
            for cells in [0, 2, 6, 9, 12, 16, 24, 48] {
                for frames in [0, 1, 4] {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    assert_eq!(
                        portable(&module, &arguments, limits),
                        portable(&ordinary, &arguments, limits),
                        "n={n}, limits={limits:?}"
                    );
                    assert_eq!(*original, [Value::I64(9), Value::I64(10)]);
                }
            }
        }
    }
}

#[test]
fn custom_boolean_recipe_keeps_lazy_syntax_and_has_no_hidden_eager_call_boundary() {
    let definition = Definition {
        word: "guard".into(),
        params: vec![ScalarType::Bool, ScalarType::Bool],
        result: ScalarType::Bool,
        body: Template::Binary {
            kind: Binary::And,
            left: Box::new(Template::Param { index: 0 }),
            right: Box::new(Template::Param { index: 1 }),
        },
    };
    let dictionary = Dictionary::new(&[definition]).unwrap();
    let source = format!(
        "wk1 {} v2 pub calc 1 flag bool bool do 1 missing bool 1 ret guard flag missing",
        dictionary.header()
    );
    let module =
        Module::compile_wordcode(&source, &dictionary, ExpansionLimits::default()).unwrap();
    assert_eq!(
        portable(&module, &[Value::Bool(false)], Limits::default()),
        Ok(Value::Bool(false))
    );
    assert_eq!(
        portable(&module, &[Value::Bool(true)], Limits::default()),
        Err("read of uninitialized local".into())
    );
    // The ordinary dictionary encoding never introduces a function call or an
    // eager argument-evaluation promise. Its full digest must be supplied.
    assert!(
        Module::compile_wordcode(
            &source.replace(dictionary.header(), "core"),
            &dictionary,
            ExpansionLimits::default()
        )
        .is_err()
    );
}

#[test]
fn function_local_type_obligations_cannot_leak_between_declarations() {
    let module = core(
        "wk1 core v2 fn floats 1 x float float ret root x \
         pub calc 1 x int int ret call later 1 up x \
         fn later 1 x int int ret twice x",
    );
    assert_eq!(
        portable(&module, &[Value::I64(20)], Limits::default()),
        Ok(Value::I64(42))
    );
    assert!(
        Module::compile_wordcode(
            "wk1 core v2 fn floats 1 x float float ret root x \
         pub calc 1 x bool int ret up x",
            &Dictionary::core(),
            ExpansionLimits::default()
        )
        .is_err()
    );
}
