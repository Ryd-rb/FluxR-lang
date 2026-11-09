//! Source ledger for recursive nominal calls and their portable replays.
use super::{BOUNDS, Budget, CELLS, FRAMES, FUEL, MATH, PAYLOAD, Replay, fields, limits, point};
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Binding, Invocation, Limits, Module, Type, Value, Workspace};
use std::sync::Arc;

const SOURCE: &str = include_str!("recursive_method_safety.fluxr");
const FSM: &[u8] = include_bytes!("../../tests/fixtures/recursive-method/fold.fsm");
const FXH: &[u8] = include_bytes!("../../tests/fixtures/recursive-method/fold.fxh");
const EXPORT: &str = "fold";

#[derive(Clone, Copy, Debug)]
struct Case {
    n: i64,
    nested: bool,
    index: u64,
    divisor: i64,
    scale: Option<u64>,
    initial: i64,
}

const NESTED: Case = Case {
    n: 3,
    nested: true,
    index: 0,
    divisor: 3,
    scale: Some(2),
    initial: 7,
};
const SELF: Case = Case {
    nested: false,
    ..NESTED
};

impl Case {
    fn inputs(self) -> [Value; 3] {
        let snapshot = point(self.initial, 60);
        let pool = Value::List(vec![snapshot.clone()].into());
        [pool.clone(), snapshot, pool]
    }

    fn args(self) -> [Value; 5] {
        [
            Value::I64(self.n),
            Value::Bool(self.nested),
            Value::U64(self.index),
            Value::I64(self.divisor),
            self.scale.map_or(Value::Variant(0, None), |n| {
                Value::Variant(1, Some(Value::U64(n).into()))
            }),
        ]
    }

    fn expected(self, cap: Limits) -> Result<Value, &'static str> {
        let mut budget = Budget(cap);
        // Visit both pools and the snapshot separately, even with shared Arcs.
        budget.cells(16 + usize::from(self.scale.is_some()))?;
        budget.enter(1, 5)?;
        budget.fuel(9)?; // Point receiver, five arguments, dispatch.

        let mut depth = 2;
        let mut n = self.n;
        loop {
            budget.enter(depth, 6)?;
            budget.fuel(4)?; // Compare n with zero and branch.
            if n <= 0 {
                break;
            }
            budget.fuel(9)?; // Receiver, n - 1, four forwarded arguments, dispatch.
            n -= 1;
            depth += 1;
        }

        budget.fuel(2)?; // Test whether to dispatch to Twin in the base frame.
        let mut value = self.initial;
        let mut detached = false;
        if self.nested {
            budget.fuel(3)?; // Resolve the indexed receiver before its arguments.
            if self.index != 0 {
                return Err(BOUNDS);
            }
            budget.fuel(3)?;
            let delta = 12_i64.checked_div(self.divisor).ok_or(MATH)?;
            budget.fuel(2)?;
            let scale = self.scale.ok_or(PAYLOAD)?;
            budget.fuel(1)?;
            budget.enter(depth + 1, 3)?;

            budget.fuel(4)?; // Twin's audit division can fail before the copy.
            60_u64.checked_div(scale).ok_or(MATH)?;
            budget.fuel(1)?;
            budget.cells(2)?; // Twin's local audit write detaches from the host.
            budget.fuel(4)?;
            value = value.checked_sub(delta).ok_or(MATH)?;
            budget.fuel(2)?; // Twin return and Point's write.
            budget.cells(2)?; // The base Point write detaches independently.
            detached = true;
            budget.fuel(1)?; // Jump over the empty else arm.
        }
        budget.fuel(2)?; // Load self and return from the base frame.

        for caller in 1..=self.n {
            budget.fuel(5)?; // Store the returned Point, then add the retained n.
            value = value.checked_add(caller).ok_or(MATH)?;
            budget.fuel(1)?;
            if !detached {
                budget.cells(2)?; // Only the first write copies the aliased Point.
                detached = true;
            }
            budget.fuel(2)?; // Load self and return to the suspended caller.
        }
        budget.fuel(2)?; // Read snapshot and construct Outcome.
        budget.cells(3)?;
        budget.fuel(1)?;
        budget.cells(7)?; // Admit the returned Outcome and both Points.
        Ok(Value::Record(
            [point(value, 60), point(self.initial, 60)].into(),
        ))
    }

    fn check_aliases(self, inputs: &[Value; 3], output: &Value) {
        let [changed, original] = &**fields(output) else {
            panic!("expected Outcome");
        };
        let snapshot = fields(&inputs[1]);
        assert!(Arc::ptr_eq(fields(original), snapshot));
        assert_eq!(
            Arc::ptr_eq(fields(changed), snapshot),
            self.n <= 0 && !self.nested
        );
    }
}

fn check_refusal(source: &str, expected: &str) {
    let Err(error) = Module::compile_compact(source) else {
        panic!("invalid recursive method compiled");
    };
    assert_eq!(error.to_string(), expected);
}

pub(crate) fn check_recursive_method_safety() -> usize {
    let compact = Module::compile_compact(SOURCE).unwrap();
    let dotted = Module::compile_compact(&SOURCE.replace("(m self fold", "(self.fold")).unwrap();
    let decoded = Module::decode(FSM).unwrap();
    for module in [&compact, &dotted, &decoded] {
        assert_eq!(module.encode(), FSM);
        assert_eq!(Hir::from_structured(module.clone()).encode().unwrap(), FXH);
    }
    let hir = Hir::decode(FXH).unwrap();
    assert_eq!(hir.encode().unwrap(), FXH);
    let checked_source = Module::compile(hir.source().unwrap()).unwrap();
    let Artifact::Structured(restored) = hir.into_artifact() else {
        panic!("recursive method FXH lost its structured origin");
    };
    for module in [&checked_source, &restored] {
        assert_eq!(module.encode(), FSM);
    }
    assert!(!compact.prepare_native(EXPORT));
    assert_eq!(
        compact.emit_native_rust(EXPORT).unwrap_err().to_string(),
        "unsupported native AOT export fold"
    );
    check_refusal(
        &SOURCE.replace("(input snapshot)", "(at (input twins) 0u)"),
        r#"expected Record("Point"), got Record("Twin")"#,
    );

    let grants = [
        Binding {
            name: "points".into(),
            ty: Type::List(Box::new(Type::Record("Point".into()))),
        },
        Binding {
            name: "snapshot".into(),
            ty: Type::Record("Point".into()),
        },
        Binding {
            name: "twins".into(),
            ty: Type::List(Box::new(Type::Record("Twin".into()))),
        },
    ];
    let mut replay = Replay::new(compact, EXPORT, &[], &grants);
    let mut workspace = Workspace::default();
    let mut count = 0;
    let mut check = |case: Case, cap: Limits| {
        let inputs = case.inputs();
        let args = case.args();
        let expected = case.expected(cap);
        let inspect = |output: &Value| case.check_aliases(&inputs, output);
        replay.check_routes_labeled(
            &inputs,
            &args,
            cap,
            expected.clone(),
            format_args!("{case:?}"),
            inspect,
        );
        for module in [&checked_source, &restored] {
            let actual = module
                .run_portable_with(
                    Invocation {
                        export: EXPORT,
                        inputs: &inputs,
                        arguments: &args,
                        limits: cap,
                    },
                    &mut workspace,
                )
                .map_err(|error| error.to_string());
            assert_eq!(
                actual.as_ref().map_err(String::as_str),
                expected.as_ref().map_err(|&error| error),
                "FXH {case:?} {cap:?}"
            );
            if let Ok(output) = actual {
                inspect(&output);
            }
        }
        let [Value::List(points), snapshot, Value::List(twins)] = &inputs else {
            panic!("expected Point and Twin pools");
        };
        assert_eq!(*snapshot, point(case.initial, 60));
        assert!(Arc::ptr_eq(fields(&points[0]), fields(snapshot)));
        assert!(Arc::ptr_eq(fields(&twins[0]), fields(snapshot)));
        assert_eq!(Arc::strong_count(fields(snapshot)), 2);
        count += 1;
    };

    for case in [
        Case {
            n: i64::MIN,
            ..SELF
        },
        Case { n: 0, ..NESTED },
        SELF,
        NESTED,
        Case {
            index: 1,
            divisor: 0,
            scale: None,
            ..NESTED
        },
        Case {
            divisor: 0,
            scale: None,
            ..NESTED
        },
        Case {
            scale: None,
            ..NESTED
        },
        Case {
            scale: Some(0),
            ..NESTED
        },
        Case {
            initial: i64::MIN,
            ..NESTED
        },
        Case {
            initial: i64::MAX,
            ..SELF
        },
    ] {
        for fuel in 0..=105 {
            check(case, limits(fuel, 70, 6));
        }
        for cells in 0..70 {
            check(case, limits(105, cells, 6));
        }
        for frames in 0..6 {
            check(case, limits(105, 70, frames));
        }
        for cells in [0, 17, 23, 44, 51, 70] {
            for frames in [0, 4, 5, 6] {
                for fuel in [0, 9, 57, 60, 62, 63] {
                    check(case, limits(fuel, cells, frames));
                }
            }
        }
    }
    for (case, cap, trap) in [
        (SELF, limits(82, 63, 5), FUEL),
        (SELF, limits(83, 62, 5), CELLS),
        (SELF, limits(83, 63, 4), FRAMES),
        (NESTED, limits(103, 69, 6), FUEL),
        (NESTED, limits(104, 68, 6), CELLS),
        (NESTED, limits(104, 69, 5), FRAMES),
        (NESTED, limits(62, 51, 5), FUEL),
        (NESTED, limits(63, 51, 5), FRAMES),
        (NESTED, limits(63, 51, 6), CELLS),
        (
            Case {
                n: i64::MAX,
                ..SELF
            },
            limits(105, 70, 6),
            FRAMES,
        ),
        (Case { n: 1024, ..SELF }, limits(21_523, 7210, 1026), FUEL),
        (Case { n: 1024, ..SELF }, limits(21_524, 7209, 1026), CELLS),
        (Case { n: 1024, ..SELF }, limits(21_524, 7210, 1025), FRAMES),
    ] {
        assert_eq!(case.expected(cap), Err(trap), "{case:?} {cap:?}");
        check(case, cap);
    }
    for (case, cap, value) in [
        (SELF, limits(83, 63, 5), 13),
        (NESTED, limits(104, 69, 6), 9),
        (
            Case { n: 1024, ..SELF },
            limits(21_524, 7210, 1026),
            524_807,
        ),
    ] {
        assert_eq!(
            case.expected(cap),
            Ok(Value::Record([point(value, 60), point(7, 60)].into()))
        );
        check(case, cap);
    }
    count
}
