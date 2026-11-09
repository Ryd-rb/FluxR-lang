//! Independent ledger for nominal method receivers and ordered argument effects.
use super::{Budget, CELLS, FRAMES, FUEL, MATH, Replay};
use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};
use std::sync::Arc;

const SOURCE: &str = include_str!("method_call_order.fluxr");
const BOUNDS: &str = "list index out of bounds";
const PAYLOAD: &str = "enum payload does not match variant";

#[path = "method_edit_isolation_checks.rs"]
mod method_edit_isolation;
pub(crate) use method_edit_isolation::check_method_edit_isolation;
#[path = "method_artifact_safety_checks.rs"]
mod method_artifact_safety;
pub(crate) use method_artifact_safety::check_method_artifact_safety;

#[derive(Clone, Copy, Debug)]
struct Case {
    index: u64,
    divisor: i64,
    scale: Option<u64>,
    initial: i64,
}

const GOOD: Case = Case {
    index: 0,
    divisor: 3,
    scale: Some(2),
    initial: 7,
};
const POST_WRITE: Case = Case {
    scale: Some(0),
    ..GOOD
};

fn point(value: i64, audit: u64) -> Value {
    Value::Record([Value::I64(value), Value::U64(audit)].into())
}

impl Case {
    fn inputs(self) -> [Value; 2] {
        let snapshot = point(self.initial, 60);
        [Value::List(vec![snapshot.clone()].into()), snapshot]
    }

    fn args(self, projected: bool, snapshot: &Value) -> [Value; 3] {
        let receiver = if projected {
            Value::Record([snapshot.clone()].into())
        } else {
            Value::U64(self.index)
        };
        let scale = self.scale.map_or(Value::Variant(0, None), |n| {
            Value::Variant(1, Some(Value::U64(n).into()))
        });
        [receiver, Value::I64(self.divisor), scale]
    }

    fn expected(
        self,
        projected: bool,
        limits: Limits,
        subtract: bool,
    ) -> Result<Value, &'static str> {
        let mut budget = Budget(limits);
        // Pool and snapshot visit the shared Point separately. Holder adds
        // three visits compared to the scalar index. Scale has an optional child.
        budget.cells(10 + 3 * usize::from(projected) + usize::from(self.scale.is_some()))?;
        budget.enter(1, 3)?;

        budget.fuel(if projected { 2 } else { 3 })?;
        if !projected && self.index != 0 {
            return Err(BOUNDS);
        }
        budget.fuel(3)?; // Literal, divisor, division.
        let delta = 12_i64.checked_div(self.divisor).ok_or(MATH)?;
        budget.fuel(2)?; // Scale load and payload access.
        let scale = self.scale.ok_or(PAYLOAD)?;
        budget.fuel(1)?; // Dispatch, then the method's frame and slots.
        budget.enter(2, 3)?;

        budget.fuel(4)?; // Read the value and delta, then apply arithmetic.
        let value = if subtract {
            self.initial.checked_sub(delta)
        } else {
            self.initial.checked_add(delta)
        }
        .ok_or(MATH)?;
        budget.fuel(1)?;
        budget.cells(2)?; // First write detaches the two-field Point.
        budget.fuel(4)?;
        let audit = 60_u64.checked_div(scale).ok_or(MATH)?;
        budget.fuel(3)?; // Second write, load self, return from update.
        budget.fuel(2)?; // Load the original alias and construct Outcome.
        budget.cells(3)?;
        budget.fuel(1)?; // Return from the export.
        budget.cells(7)?; // Outcome and its two Points.
        Ok(Value::Record(
            [point(value, audit), point(self.initial, 60)].into(),
        ))
    }
}

const fn limits(instructions: u64, cells: usize, frames: usize) -> Limits {
    Limits {
        instructions,
        cells,
        frames,
    }
}

const INDEXED: Limits = limits(24, 31, 2);
const PROJECTED: Limits = limits(23, 34, 2);

fn fields(value: &Value) -> &Arc<[Value]> {
    let Value::Record(fields) = value else {
        panic!("expected record")
    };
    fields
}

fn check_aliases(case: Case, projected: bool, inputs: &[Value; 2], args: &[Value; 3]) {
    let Value::List(pool) = &inputs[0] else {
        panic!("expected pool")
    };
    let snapshot = &inputs[1];
    assert_eq!(*snapshot, point(case.initial, 60));
    assert!(Arc::ptr_eq(fields(&pool[0]), fields(snapshot)));
    if projected {
        assert!(Arc::ptr_eq(fields(&fields(&args[0])[0]), fields(snapshot)));
    }
}

fn check_result(value: &Value, snapshot: &Value) {
    let [changed, original] = &**fields(value) else {
        panic!("expected Outcome")
    };
    assert!(!Arc::ptr_eq(fields(changed), fields(snapshot)));
    assert!(Arc::ptr_eq(fields(original), fields(snapshot)));
}

fn check_replacement(module: Module, grants: &[Binding]) {
    let original = module.encode();
    let edit = SOURCE.replace("(+ self.value delta)", "(- self.value delta)");
    let replacement = Module::compile_compact(&edit).unwrap();
    let bytes = replacement.encode();
    assert_ne!(bytes, original);
    let mut slot = ModuleSlot::new(module, grants).unwrap();
    let inputs = GOOD.inputs();
    let good = GOOD.args(false, &inputs[1]);
    let failing = POST_WRITE.args(false, &inputs[1]);
    let call = |arguments, limits| Invocation {
        export: "indexed",
        inputs: &inputs,
        arguments,
        limits,
    };
    let old = GOOD.expected(false, INDEXED, false).unwrap();
    for (case, args, cap, trap) in [
        (GOOD, &good, limits(8, 15, 1), FUEL),
        (GOOD, &good, limits(9, 15, 1), FRAMES),
        (GOOD, &good, limits(9, 18, 2), CELLS),
        (GOOD, &good, limits(14, 20, 2), CELLS),
        (POST_WRITE, &failing, limits(18, 21, 2), MATH),
        (GOOD, &good, limits(24, 30, 2), CELLS),
    ] {
        assert_eq!(case.expected(false, cap, true), Err(trap));
        let error = slot
            .inject(0, Module::decode(&bytes).unwrap(), Some(call(args, cap)))
            .unwrap_err();
        assert_eq!(error.to_string(), trap);
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
        let actual = slot.run(call(&good, INDEXED)).unwrap();
        assert_eq!(actual, old);
        check_result(&actual, &inputs[1]);
        check_aliases(case, false, &inputs, args);
    }
    assert_eq!(
        slot.inject(0, replacement, Some(call(&good, INDEXED)))
            .unwrap(),
        1
    );
    assert_eq!(slot.module().encode(), bytes);
    let actual = slot.run(call(&good, INDEXED)).unwrap();
    assert_eq!(actual, GOOD.expected(false, INDEXED, true).unwrap());
    assert_ne!(actual, old);
    check_result(&actual, &inputs[1]);
    check_aliases(GOOD, false, &inputs, &good);
}

pub(crate) fn check_method_call_order() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    let dotted = SOURCE.replace("(m holder.point update", "(holder.point.update");
    let dotted = Module::compile_compact(&dotted).unwrap();
    assert_eq!(module.encode(), dotted.encode());
    for invalid in [
        SOURCE.replace("(m holder.point update", "(m holder update"),
        SOURCE.replace("update (/ 12 divisor)", "update 3u"),
    ] {
        assert!(Module::compile_compact(&invalid).is_err());
    }
    let point_type = Type::Record("Point".into());
    let grants = [
        Binding {
            name: "pool".into(),
            ty: Type::List(Box::new(point_type.clone())),
        },
        Binding {
            name: "snapshot".into(),
            ty: point_type,
        },
    ];

    let mut count = 0;
    for (export, projected, exact) in [("indexed", false, INDEXED), ("projected", true, PROJECTED)]
    {
        assert!(!module.prepare_native(export));
        assert!(module.emit_native_rust(export).is_err());
        let mut replay = Replay::new(module.clone(), export, &[], &grants);
        let mut source = ModuleSlot::new(module.clone(), &grants).unwrap();
        assert_eq!(source.inject(0, dotted.clone(), None).unwrap(), 1);
        let mut workspace = Workspace::default();
        let cases = [
            GOOD,
            Case { divisor: 0, ..GOOD },
            Case {
                scale: None,
                ..GOOD
            },
            Case {
                divisor: 0,
                scale: None,
                ..GOOD
            },
            POST_WRITE,
            Case {
                initial: i64::MAX,
                ..GOOD
            },
        ];
        let bounds = cases[..4].iter().map(|case| Case { index: 1, ..*case });
        for case in cases.into_iter().chain(bounds.filter(|_| !projected)) {
            let inputs = case.inputs();
            let args = case.args(projected, &inputs[1]);
            for frames in 0..=2 {
                for cells in 0..=exact.cells + 1 {
                    for instructions in 0..=exact.instructions + 1 {
                        let cap = limits(instructions, cells, frames);
                        let expected = case.expected(projected, cap, false);
                        let inspect = |value: &Value| check_result(value, &inputs[1]);
                        replay.check_routes_labeled(
                            &inputs,
                            &args,
                            cap,
                            expected.clone(),
                            format_args!("{case:?}"),
                            inspect,
                        );
                        let actual = source
                            .run_with(
                                Invocation {
                                    export,
                                    inputs: &inputs,
                                    arguments: &args,
                                    limits: cap,
                                },
                                &mut workspace,
                            )
                            .map_err(|e| e.to_string());
                        assert_eq!(
                            actual.as_ref().map_err(String::as_str),
                            expected.as_ref().map_err(|&trap| trap),
                            "source injection {export} {case:?} {cap:?}",
                        );
                        if let Ok(value) = &actual {
                            inspect(value);
                        }
                        check_aliases(case, projected, &inputs, &args);
                    }
                }
            }
        }
        count += replay.count;
    }
    check_replacement(module, &grants);
    count
}
