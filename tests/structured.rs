use fluxr::structured::{
    Binding, Invocation, Limits, Module, ModuleSlot, Source, Type, Value, Workspace,
};
use serde_json::{Value as Json, json};
use std::hint::black_box;
use std::time::Instant;

#[test]
fn version_two_arg_min_preserves_first_ties_and_charges_each_value() {
    let mut source = body(
        json!([]),
        json!([ret(json!({
            "op":"arg_min_f32", "list":{"op":"list","element":"f32","items":[
                {"op":"f32","bits":2_f32.to_bits()},
                {"op":"f32","bits":1_f32.to_bits()},
                {"op":"f32","bits":1_f32.to_bits()}
            ]}
        }))]),
        json!("u64"),
    );
    assert!(Module::compile_text(&source.to_string()).is_err());
    source["version"] = json!(2);
    let module = compile(source);
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

const COUNTER: &str = include_str!("fixtures/structured_counter.fluxr");

fn compile(module: Json) -> Module {
    Module::compile_text(&module.to_string()).unwrap()
}

fn body(locals: Json, code: Json, result: Json) -> Json {
    json!({"version":1,"functions":[{"name":"run","export":true,
        "locals":locals,"result":result,"body":code}]})
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

fn int(value: i64) -> Json {
    json!({"op":"i64","value":value})
}
fn local(name: &str) -> Json {
    json!({"op":"local","name":name})
}
fn binary(kind: &str, left: Json, right: Json) -> Json {
    json!({"op":"binary","kind":kind,"left":left,"right":right})
}
fn ret(value: Json) -> Json {
    json!({"op":"return","value":value})
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
fn typed_text_and_portable_encoding_agree_and_execute_methods() {
    let from_text = Module::compile_text(COUNTER).unwrap();
    let typed = serde_json::from_str::<Source>(COUNTER).unwrap();
    let encoded = Module::compile(typed).unwrap().encode();
    assert_eq!(from_text.encode(), encoded);
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
    let initial = Module::compile_text(COUNTER).unwrap();
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

    let next = COUNTER.replace("\"value\":2}}}", "\"value\":4}}}");
    assert_ne!(next, COUNTER);
    assert_eq!(
        slot.inject(
            0,
            Module::compile_text(&next).unwrap(),
            Some(tick_args(&continued))
        )
        .unwrap(),
        1
    );
    assert_eq!(slot.run(tick_args(&continued)).unwrap(), state(82));
    assert!(
        slot.inject(0, Module::compile_text(COUNTER).unwrap(), None)
            .is_err()
    );

    let mut foreign: Json = serde_json::from_str(COUNTER).unwrap();
    foreign["inputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"hidden","ty":"i64"}));
    assert!(slot.inject(1, compile(foreign), None).is_err());

    let mut looped: Json = serde_json::from_str(COUNTER).unwrap();
    looped["functions"][2]["body"]
        .as_array_mut()
        .unwrap()
        .insert(
            0,
            json!({"op":"while","test":{"op":"bool","value":true},"body":[]}),
        );
    let error = slot
        .inject(1, compile(looped), Some(tick_args(&continued)))
        .unwrap_err();
    assert_eq!(error.to_string(), "module instruction budget exhausted");
    assert_eq!(slot.revision(), 1);
    assert_eq!(slot.run(tick_args(&continued)).unwrap(), state(82));
    assert_eq!(continued[0], state(37));
}

#[test]
fn loops_can_move_values_between_stacks_and_return_enum_payloads() {
    let mut source = body(
        json!([{"name":"left","ty":{"list":"i64"}},{"name":"right","ty":{"list":"i64"}},
               {"name":"top","ty":"i64"}]),
        json!([
            {"op":"set","local":"left","value":{"op":"list","element":"i64","items":[int(1),int(2),int(3)]}},
            {"op":"set","local":"right","value":{"op":"list","element":"i64"}},
            {"op":"while","test":{"op":"unary","kind":"not","value":{"op":"empty","list":local("left")}},"body":[
                {"op":"set","local":"top","value":{"op":"last","list":local("left")}},
                {"op":"pop","local":"left"},
                {"op":"if","test":binary("eq",local("top"),int(2)),"then":[{"op":"continue"}]},
                {"op":"push","local":"right","value":local("top")}
            ]},
            ret(json!({"op":"variant","name":"Outcome","variant":"Ok","payload":local("right")}))
        ]),
        json!({"enum":"Outcome"}),
    );
    source["enums"] = json!([{"name":"Outcome","variants":[
        {"name":"None"},{"name":"Ok","payload":{"list":"i64"}}
    ]}]);
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
    let module = compile(json!({"version":1,"functions":[{
        "name":"sum","export":true,"params":[{"name":"n","ty":"i64"}],"result":"i64",
        "body":[{"op":"if","test":binary("eq",local("n"),int(0)),
            "then":[ret(int(0))],
            "otherwise":[ret(binary("add",local("n"),json!({
                "op":"call","name":"sum","args":[binary("sub",local("n"),int(1))]
            })))]}]
    }]}));
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
    let trap = binary("eq", binary("div", int(1), int(0)), int(0));
    let module = compile(body(
        json!([]),
        json!([ret(binary("and", json!({"op":"bool","value":false}), trap))]),
        json!("bool"),
    ));
    assert_eq!(
        run(&module, "run", &[], Limits::default()).unwrap(),
        Value::Bool(false)
    );

    let module = compile(body(
        json!([]),
        json!([ret(binary("add", int(i64::MAX), int(1)))]),
        json!("i64"),
    ));
    assert!(run(&module, "run", &[], Limits::default()).is_err());

    let module = compile(body(
        json!([]),
        json!([ret(binary(
            "add",
            json!({"op":"f32","bits":0x7f80_0001_u32}),
            json!({"op":"f32","bits":0})
        ))]),
        json!("f32"),
    ));
    assert_eq!(
        run(&module, "run", &[], Limits::default()).unwrap(),
        Value::F32(0x7fc0_0000)
    );
}

#[test]
fn input_validation_and_allocation_are_metered() {
    let module = compile(body(
        json!([{"name":"stack","ty":{"list":"i64"}}]),
        json!([
            {"op":"set","local":"stack","value":{"op":"list","element":"i64"}},
            {"op":"while","test":{"op":"bool","value":true},"body":[{"op":"push","local":"stack","value":int(1)}]},
            ret(int(0))
        ]),
        json!("i64"),
    ));
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
    let counter = Module::compile_text(COUNTER).unwrap();
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
    for source in [
        body(json!([]), json!([{"op":"break"}]), json!("unit")),
        body(json!([]), json!([]), json!("unit")),
        body(json!([]), json!([ret(int(1))]), json!("bool")),
        body(json!([]), json!([ret(local("hidden"))]), json!("i64")),
        body(json!([]), json!([ret(int(1)), ret(int(2))]), json!("i64")),
    ] {
        assert!(Module::compile_text(&source.to_string()).is_err());
    }
    let mut recursive = body(json!([]), json!([ret(int(1))]), json!("i64"));
    recursive["records"] = json!([{"name":"Cycle","fields":[
        {"name":"nested","ty":{"list":{"record":"Cycle"}}}
    ]}]);
    assert!(
        Module::compile_text(&recursive.to_string())
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

    let module = Module::compile_text(COUNTER).unwrap();
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
        black_box(Module::compile_text(black_box(COUNTER)).unwrap());
    }
    println!("prepare_us {:.2}", start.elapsed().as_secs_f64() * 1e3);
}
