use fluxr::structured::{
    Binding, Invocation, Limits, Module, ModuleSlot, Source, Type, Value, Workspace,
};
use std::hint::black_box;
use std::time::Instant;

#[test]
fn adjacent_arithmetic_and_store_keep_trap_precedence() {
    let module =
        Module::compile_compact("v2 (pub calc (x:i64) i64 (y:i64) (= y (+ x 1)) (ret y))").unwrap();
    let run = |x, instructions| {
        module.run(Invocation {
            export: "calc",
            inputs: &[],
            arguments: &[Value::I64(x)],
            limits: Limits {
                instructions,
                ..Limits::default()
            },
        })
    };
    assert_eq!(
        run(i64::MAX, 3).unwrap_err().to_string(),
        "invalid or overflowing integer arithmetic"
    );
    assert_eq!(
        run(0, 3).unwrap_err().to_string(),
        "module instruction budget exhausted"
    );
    assert_eq!(run(0, 6).unwrap(), Value::I64(1));
}

#[test]
fn version_two_arg_min_preserves_first_ties_and_charges_each_value() {
    let body = "(pub run () u64 () (ret (minidx (list f32 2f 1f 1f))))";
    assert!(Module::compile_compact(&format!("v1 {body}")).is_err());
    let module = compile(&format!("v2 {body}"));
    let encoded = module.encode();
    let decoded = Module::decode(&encoded).unwrap();
    assert_eq!(decoded.encode(), encoded);
    assert_eq!(
        run(&decoded, "run", &[], Limits::default()).unwrap(),
        Value::U64(1)
    );
    assert!(
        run(
            &decoded,
            "run",
            &[],
            Limits {
                instructions: 7,
                ..Limits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn arg_min_of_uninitialized_local_traps_after_the_load_charge() {
    let module =
        Module::compile_compact("v2 (pub run () u64 (values:[f32]) (ret (minidx values)))")
            .unwrap();
    for (instructions, expected) in [
        (0, "module instruction budget exhausted"),
        (1, "read of uninitialized local"),
    ] {
        let limits = Limits {
            instructions,
            ..Limits::default()
        };
        assert_eq!(
            run(&module, "run", &[], limits).unwrap_err().to_string(),
            expected
        );
    }
}

#[test]
fn native_budget_covers_list_copy_and_rejects_unbounded_control_flow() {
    let source = "v2
(pub append (values:[u64]) [u64] (copy:[u64])
  (= copy values)
  (push copy 1u)
  (ret copy))
(pub repeat (active:bool) bool ()
  (while active ())
  (ret false))
";
    let module = Module::compile_compact(source).unwrap();
    let original = Value::List(vec![Value::U64(2), Value::U64(3)].into());
    let arguments = [original.clone()];
    let bound = module.native_ceiling("append", 3).unwrap();
    assert_eq!(
        run(&module, "append", &arguments, bound).unwrap(),
        Value::List(vec![Value::U64(2), Value::U64(3), Value::U64(1)].into())
    );
    assert_eq!(original, arguments[0]);
    assert!(module.native_ceiling("repeat", 1).is_none());
    let bound = module.native_loop_ceiling("repeat", 1, 4).unwrap();
    assert_eq!(
        run(&module, "repeat", &[Value::Bool(false)], bound).unwrap(),
        Value::Bool(false)
    );
    assert!(run(&module, "repeat", &[Value::Bool(true)], bound).is_err());
    assert!(
        module
            .native_loop_ceiling("repeat", 1, usize::MAX)
            .is_none()
    );
}

const COUNTER: &str = include_str!("../examples/structured-counter.fluxr");

fn compile(source: &str) -> Module {
    Module::compile_compact(source).unwrap()
}

fn run(
    module: &Module,
    export: &str,
    args: &[Value],
    limits: Limits,
) -> Result<Value, fluxr::Error> {
    module.run(Invocation {
        export,
        inputs: &[],
        arguments: args,
        limits,
    })
}

fn state(total: i64) -> Value {
    Value::Record([Value::I64(total)].into())
}

fn tick_args(args: &[Value]) -> Invocation<'_> {
    static INPUT: [Value; 1] = [Value::I64(3)];
    Invocation {
        export: "tick",
        inputs: &INPUT,
        arguments: args,
        limits: Limits::default(),
    }
}

#[test]
fn typed_and_compact_source_preserve_released_fsm_bytes() {
    let from_text = Module::compile_compact(COUNTER).unwrap();
    let typed = Source::parse_compact(COUNTER).unwrap();
    let encoded = Module::compile(typed).unwrap().encode();
    assert_eq!(from_text.encode(), encoded);
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(&encoded)),
        "194e0cc56e112c79a45b7d1d4650a0bedc0cf6da1cd8b88c56dfa80f965d49e5"
    );
    let decoded = Module::decode(&encoded).unwrap();
    assert_eq!(decoded.encode(), encoded);
    let initial = state(10);
    let args = [initial.clone(), Value::I64(6)];
    assert_eq!(decoded.run(tick_args(&args)).unwrap(), state(37));
    assert_eq!(initial, state(10));
    let mut bad = encoded;
    bad[4] = 2;
    assert!(Module::decode(&bad).is_err());
}

#[test]
fn slot_keeps_revision_and_state_on_bad_interface_stale_or_exhausted_candidate() {
    let initial = Module::compile_compact(COUNTER).unwrap();
    assert!(ModuleSlot::new(initial.clone(), &[]).is_err());
    let grants = [Binding {
        name: "bonus".into(),
        ty: Type::I64,
    }];
    let mut slot = ModuleSlot::new(initial, &grants).unwrap();
    let args = [state(10), Value::I64(6)];
    let result = slot.run(tick_args(&args)).unwrap();
    let continued = [result, Value::I64(6)];
    assert_eq!(slot.run(tick_args(&continued)).unwrap(), state(64));

    let next = COUNTER.replace("(ret (* i 2))", "(ret (* i 4))");
    assert_ne!(next, COUNTER);
    assert_eq!(
        slot.inject(
            0,
            Module::compile_compact(&next).unwrap(),
            Some(tick_args(&continued))
        )
        .unwrap(),
        1
    );
    assert_eq!(slot.run(tick_args(&continued)).unwrap(), state(82));
    assert!(
        slot.inject(0, Module::compile_compact(COUNTER).unwrap(), None)
            .is_err()
    );

    let foreign = COUNTER.replace("(inputs bonus:i64)", "(inputs bonus:i64 hidden:i64)");
    assert!(slot.inject(1, compile(&foreign), None).is_err());

    let looped = COUNTER.replace(
        "(ret (state.advance count))",
        "(while true ()) (ret (state.advance count))",
    );
    let error = slot
        .inject(1, compile(&looped), Some(tick_args(&continued)))
        .unwrap_err();
    assert_eq!(error.to_string(), "module instruction budget exhausted");
    assert_eq!(slot.revision(), 1);
    assert_eq!(slot.run(tick_args(&continued)).unwrap(), state(82));
    assert_eq!(continued[0], state(37));
}

#[test]
fn loops_can_move_values_between_stacks_and_return_enum_payloads() {
    let source = "v1
        (enum Outcome None Ok:[i64])
        (pub run () #Outcome (left:[i64] right:[i64] top:i64)
          (= left (list i64 1 2 3))
          (= right (list i64))
          (while (! (empty left)) (
            (= top (last left))
            (pop left)
            (if (== top 2) ((continue)))
            (push right top)))
          (ret (tag Outcome.Ok right)))";
    assert_eq!(
        run(&compile(source), "run", &[], Limits::default()).unwrap(),
        Value::Variant(
            1,
            Some(Value::List(vec![Value::I64(3), Value::I64(1)].into()).into())
        )
    );
}

#[test]
fn calls_recurse_and_keep_the_callers_operands() {
    let module = compile(
        "v1
        (pub sum (n:i64) i64 ()
          (if (== n 0) ((ret 0)) ((ret (+ n (call sum (- n 1)))))))",
    );
    let args = [Value::I64(10)];
    assert_eq!(
        run(&module, "sum", &args, Limits::default()).unwrap(),
        Value::I64(55)
    );
    assert_eq!(
        run(
            &module,
            "sum",
            &args,
            Limits {
                frames: 8,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .to_string(),
        "module frame limit exceeded"
    );
}

#[test]
fn operators_short_circuit_and_arithmetic_is_deterministic() {
    let module = compile("v1 (pub run () bool () (ret (&& false (== (/ 1 0) 0))))");
    assert_eq!(
        run(&module, "run", &[], Limits::default()).unwrap(),
        Value::Bool(false)
    );

    let module = compile("v1 (pub run () i64 () (ret (+ 9223372036854775807 1)))");
    assert!(run(&module, "run", &[], Limits::default()).is_err());

    let module = compile("v1 (pub run () f32 () (ret (+ 0x7f800001f 0f)))");
    assert_eq!(
        run(&module, "run", &[], Limits::default()).unwrap(),
        Value::F32(0x7fc0_0000)
    );
}

#[test]
fn input_validation_and_allocation_are_metered() {
    let module = compile(
        "v1
        (pub run () i64 (stack:[i64])
          (= stack (list i64))
          (while true ((push stack 1)))
          (ret 0))",
    );
    assert_eq!(
        run(
            &module,
            "run",
            &[],
            Limits {
                cells: 16,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .to_string(),
        "module cell budget exhausted"
    );
    let counter = Module::compile_compact(COUNTER).unwrap();
    assert!(
        counter
            .run(tick_args(&[
                Value::Record([Value::Bool(false)].into()),
                Value::I64(6)
            ]))
            .is_err()
    );
    assert!(
        counter
            .run(tick_args(&[Value::I64(0), Value::I64(6)]))
            .is_err()
    );
}

#[test]
fn compiler_rejects_malformed_control_flow_types_and_interfaces() {
    for body in [
        "(pub run () unit () (break))",
        "(pub run () unit ())",
        "(pub run () bool () (ret 1))",
        "(pub run () i64 () (ret hidden))",
        "(pub run () i64 () (ret 1) (ret 2))",
    ] {
        assert!(Module::compile_compact(&format!("v1 {body}")).is_err());
    }
    let recursive = "v1 (rec Cycle nested:[Cycle]) (pub run () i64 () (ret 1))";
    assert!(
        Module::compile_compact(recursive)
            .unwrap_err()
            .to_string()
            .contains("recursive value type")
    );
}

#[test]
#[ignore = "optimized matched workload measurement"]
fn benchmark_interpreted_and_equivalent_rust() {
    fn reference(args: &[Value]) -> Value {
        let (Value::Record(fields), Value::I64(count)) = (&args[0], &args[1]) else {
            unreachable!()
        };
        let Value::I64(mut total) = fields[0] else {
            unreachable!()
        };
        for i in 0..*count {
            let change = if i % 2 == 0 {
                3
            } else {
                i.checked_mul(2).unwrap()
            };
            total = total.checked_add(change).unwrap();
        }
        state(total)
    }
    fn timed(cases: &[[Value; 2]], mut evaluate: impl FnMut(&[Value]) -> Value) -> f64 {
        let start = Instant::now();
        for _ in 0..2_000 {
            for args in black_box(cases) {
                black_box(evaluate(black_box(args)));
            }
        }
        start.elapsed().as_secs_f64() * 1e9 / (2_000 * cases.len()) as f64
    }

    let module = Module::compile_compact(COUNTER).unwrap();
    let mut workspace = Workspace::default();
    let mut interpret = |args: &[Value]| module.run_with(tick_args(args), &mut workspace).unwrap();
    for (name, counts) in [("small", [1, 4, 6, 8]), ("loop", [16, 32, 64, 128])] {
        let cases = counts.map(|n| [state(10), Value::I64(n)]);
        for args in &cases {
            assert_eq!(interpret(args), reference(args));
        }
        for pair in 0..9 {
            let (a, b) = if pair % 2 == 0 {
                (timed(&cases, reference), timed(&cases, &mut interpret))
            } else {
                let flux = timed(&cases, &mut interpret);
                (timed(&cases, reference), flux)
            };
            println!("match {name} {pair} {a:.2} {b:.2}");
        }
    }
    let start = Instant::now();
    for _ in 0..1_000 {
        black_box(Module::compile_compact(black_box(COUNTER)).unwrap());
    }
    println!("prepare_us {:.2}", start.elapsed().as_secs_f64() * 1e3);
}
