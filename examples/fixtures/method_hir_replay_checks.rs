//! Replay checked HIR against the sealed FSM revisions and the ordered-call oracle.
use super::{
    Case, GOOD, INDEXED, POST_WRITE, PROJECTED, SOURCE, check_aliases, check_result, limits, point,
};
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, Workspace};

const ORIGINAL_FSM: &[u8] = include_bytes!("../../tests/fixtures/method-hir/original.fsm");
const EDITED_FSM: &[u8] = include_bytes!("../../tests/fixtures/method-hir/edited.fsm");
const ORIGINAL_FXH: &[u8] = include_bytes!("../../tests/fixtures/method-hir/original.fxh");
const EDITED_FXH: &[u8] = include_bytes!("../../tests/fixtures/method-hir/edited.fxh");
const REFUSALS: &str = include_str!("../../tests/fixtures/method-hir/refusals.json");

fn check_refusals(wire: &[u8]) -> usize {
    let mut version = wire.to_vec();
    version[4] = 2;
    let mut kind = wire.to_vec();
    kind[5] = 2;
    let mut invalid_utf8 = wire.to_vec();
    invalid_utf8[10] = 0xff;
    let header_cases = [
        (&wire[..9], "invalid FXH envelope"),
        (&version, "unsupported FXH version 2"),
        (&kind, "unsupported FXH kind 2"),
        (
            &wire[..wire.len() - 1],
            "FXH payload size differs from envelope",
        ),
        (
            &invalid_utf8,
            "invalid utf-8 sequence of 1 bytes from index 0",
        ),
    ];
    let refuse = |bytes: &[u8], expected: &str| match Hir::decode(bytes) {
        Ok(_) => panic!("FXH accepted despite expected refusal: {expected}"),
        Err(error) => assert_eq!(error.to_string(), expected),
    };
    for (bytes, expected) in header_cases {
        refuse(bytes, expected);
    }

    let payload = std::str::from_utf8(&wire[10..]).unwrap();
    let source_cases: Vec<[String; 4]> = serde_json::from_str(REFUSALS).unwrap();
    for [id, from, to, expected] in &source_cases {
        assert_eq!(payload.matches(from).count(), 1, "{id}");
        let payload = payload.replace(from, to);
        let mut changed = wire[..6].to_vec();
        changed.extend(u32::try_from(payload.len()).unwrap().to_le_bytes());
        changed.extend(payload.as_bytes());
        refuse(&changed, expected);
    }
    header_cases.len() + source_cases.len()
}

fn check_calls(module: &Module, edited: bool) -> usize {
    let mut checked = 0;
    let mut workspace = Workspace::default();
    for (export, projected, cap) in [("indexed", false, INDEXED), ("projected", true, PROJECTED)] {
        let shift = u64::from(projected);
        let extra = 3 * usize::from(projected);
        let first_trap = Case {
            divisor: 0,
            scale: None,
            ..GOOD
        };
        let payload_trap = Case {
            scale: None,
            ..GOOD
        };
        let rows = [
            (GOOD, cap),
            (GOOD, limits(cap.instructions - 1, cap.cells, 2)),
            (GOOD, limits(cap.instructions, cap.cells - 1, 2)),
            (GOOD, limits(8 - shift, 15 + extra, 1)),
            (GOOD, limits(9 - shift, 15 + extra, 1)),
            (first_trap, limits(6 - shift, cap.cells, 2)),
            (payload_trap, limits(8 - shift, cap.cells, 2)),
            (POST_WRITE, limits(17 - shift, 21 + extra, 2)),
            (POST_WRITE, limits(18 - shift, 21 + extra, 2)),
        ];
        let receiver_trap = Case {
            index: 1,
            ..first_trap
        };
        for (case, limits) in rows
            .into_iter()
            .chain((!projected).then_some((receiver_trap, limits(3, 14, 1))))
        {
            let inputs = case.inputs();
            let arguments = case.args(projected, &inputs[1]);
            let actual = module
                .run_portable_with(
                    Invocation {
                        export,
                        inputs: &inputs,
                        arguments: &arguments,
                        limits,
                    },
                    &mut workspace,
                )
                .map_err(|error| error.to_string());
            let expected = case.expected(projected, limits, edited);
            assert_eq!(
                actual.as_ref().map_err(String::as_str),
                expected.as_ref().map_err(|&err| err),
                "{export} {case:?} {limits:?} edited={edited}"
            );
            if let Ok(result) = actual {
                check_result(&result, &inputs[1]);
            }
            check_aliases(case, projected, &inputs, &arguments);
            checked += 1;
        }
    }

    let inputs = GOOD.inputs();
    let arguments = [point(7, 60)];
    let result = module.run_portable_with(
        Invocation {
            export: "twin",
            inputs: &inputs,
            arguments: &arguments,
            limits: Limits {
                instructions: 10,
                cells: 40,
                frames: 2,
            },
        },
        &mut workspace,
    );
    assert_eq!(result.unwrap(), point(13, 60));
    checked + 1
}

pub(crate) fn check_method_hir_replay() -> usize {
    let original = format!("{SOURCE}\n{}", include_str!("method_edit_identity.fluxr"));
    let edited = original.replace("(+ self.value delta)", "(- self.value delta)");
    let mut checked = 0;
    for (source, fsm, fxh, edited) in [
        (original, ORIGINAL_FSM, ORIGINAL_FXH, false),
        (edited, EDITED_FSM, EDITED_FXH, true),
    ] {
        let module = Module::compile_compact(&source).unwrap();
        let dotted = source.replace("(m holder.point update", "(holder.point.update");
        let from_fsm = Module::decode(fsm).unwrap();
        for module in [
            module,
            Module::compile_compact(&dotted).unwrap(),
            from_fsm.clone(),
        ] {
            assert_eq!(module.encode(), fsm);
            assert_eq!(Hir::from_structured(module).encode().unwrap(), fxh);
        }
        let hir = Hir::decode(fxh).unwrap();
        assert_eq!(hir.encode().unwrap(), fxh);
        let from_hir = Module::compile(hir.source().unwrap()).unwrap();
        let Artifact::Structured(restored) = hir.into_artifact() else {
            panic!("method FXH lost its structured origin");
        };
        for module in [from_fsm, from_hir, restored] {
            assert_eq!(module.encode(), fsm);
            checked += check_calls(&module, edited);
        }
        checked += check_refusals(fxh);
    }
    checked
}
