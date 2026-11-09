//! Replay a frozen wire corpus against both readers and an active module slot.
use super::{list_effect_checks::list, shared_host_admission_checks, structured_boundary_checks};
use fluxr::Error;
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Source, Value};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const LEDGER: &str = include_str!("../../tests/fixtures/nested-payload/refusals.tsv");
const WIRE_SHA256: &str = "f689f4c4aa44bcc4410bdcbd86d31714e44ab068541a2abf6ced88b37cd81ac6";

macro_rules! fixture {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!($name, ".fluxr")),
            include_bytes!(concat!(
                "../../tests/fixtures/nested-payload/",
                $name,
                ".fsm"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../../tests/fixtures/nested-payload/",
                $name,
                ".fxh"
            ))
            .as_slice(),
        )
    };
}

fn restored(hir: Hir) -> Module {
    let Artifact::Structured(module) = hir.into_artifact() else {
        panic!("expected a structured origin");
    };
    module
}

fn with_payload(kind: &str, wire: &[u8], payload: &[u8]) -> Vec<u8> {
    if kind == "fsm" {
        structured_boundary_checks::replace_fsm_body(wire, payload)
    } else {
        let mut bytes = wire[..6].to_vec();
        bytes.extend(u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend(payload);
        bytes
    }
}

fn mutate(wire: &[u8], kind: &str, action: &str, target: &str, replacement: &str) -> Vec<u8> {
    match action {
        "set" => {
            let mut body: Json = serde_json::from_slice(&wire[6..]).unwrap();
            let field = body.pointer_mut(target).unwrap();
            let value: Json = serde_json::from_str(replacement).unwrap();
            assert_ne!(*field, value);
            *field = value;
            with_payload(kind, wire, &serde_json::to_vec(&body).unwrap())
        }
        "replace" => {
            let source = std::str::from_utf8(&wire[10..]).unwrap();
            assert_eq!(source.matches(target).count(), 1, "{target}");
            with_payload(kind, wire, source.replace(target, replacement).as_bytes())
        }
        "truncate" => {
            let end: isize = target.parse().unwrap();
            let length = if end < 0 {
                wire.len().checked_add_signed(end).unwrap()
            } else {
                usize::try_from(end).unwrap()
            };
            wire[..length].to_vec()
        }
        "byte" => {
            let mut bytes = wire.to_vec();
            bytes[target.parse::<usize>().unwrap()] = replacement.parse().unwrap();
            bytes
        }
        "append" | "pad" => {
            let mut payload = wire[if kind == "fsm" { 6 } else { 10 }..].to_vec();
            let size: usize = target.parse().unwrap();
            let length = if action == "append" {
                payload.len() + size
            } else {
                size
            };
            payload.resize(length, b' ');
            with_payload(kind, wire, &payload)
        }
        _ => panic!("unknown mutation {action}"),
    }
}

fn alias(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::List(a), Value::List(b)) => Arc::ptr_eq(a, b),
        (Value::Record(a), Value::Record(b)) => Arc::ptr_eq(a, b),
        (Value::Variant(a, Some(x)), Value::Variant(b, Some(y))) => a == b && Arc::ptr_eq(x, y),
        _ => false,
    }
}

pub(crate) fn check_nested_payload_artifact_safety() -> usize {
    let row = list([Value::I64(3), Value::I64(5)]);
    let rows = list([row.clone(), row.clone()]);
    let selected = Value::Variant(1, Some(Arc::new(row.clone())));
    let leaf = shared_host_admission_checks::leaf(Value::I64(9), Value::Bool(true));
    let tree = shared_host_admission_checks::tree(&leaf, 2, None);
    let mut digest = Sha256::new();
    let mut count = 0;
    for ((name, source, fsm, fxh), edit, export, inputs, arguments, limits, expected) in [
        (
            fixture!("list_boundary"),
            (
                "(at (at (input base) row) col)",
                "(at (at (input base) row) row)",
            ),
            "edit",
            vec![rows.clone()],
            vec![rows, Value::U64(0), Value::U64(1)],
            Limits {
                instructions: 18,
                cells: 32,
                frames: 2,
            },
            [
                list([3, 5, 5].map(Value::I64)),
                list([3, 5, 3].map(Value::I64)),
            ],
        ),
        (
            fixture!("variant_payload"),
            ("(push row 7)", "(push row 8)"),
            "edit",
            vec![selected.clone()],
            vec![selected, Value::Bool(false)],
            Limits {
                instructions: 18,
                cells: 24,
                frames: 2,
            },
            [
                list([3, 5, 7, 3].map(Value::I64)),
                list([3, 5, 8, 3].map(Value::I64)),
            ],
        ),
        (
            fixture!("shared_host_admission"),
            ("(ret 7u)", "(ret 8u)"),
            "admit",
            vec![tree.clone(), leaf.clone()],
            vec![tree, leaf],
            Limits {
                instructions: 2,
                cells: 13_318,
                frames: 1,
            },
            [Value::U64(7), Value::U64(8)],
        ),
    ] {
        let original = Module::decode(fsm).unwrap();
        assert_eq!(
            Module::compile_compact(source).unwrap().encode(),
            fsm,
            "{name}"
        );
        assert_eq!(original.encode(), fsm, "{name}");
        assert_eq!(
            Hir::from_structured(original.clone()).encode().unwrap(),
            fxh,
            "{name}"
        );
        let hir = Hir::decode(fxh).unwrap();
        assert_eq!(hir.encode().unwrap(), fxh, "{name}");
        assert_eq!(restored(hir).encode(), fsm, "{name}");

        let grants = Source::parse_compact(source).unwrap().inputs;
        let mut slot = ModuleSlot::new(original.clone(), &grants).unwrap();
        let call = || Invocation {
            export,
            inputs: &inputs,
            arguments: &arguments,
            limits,
        };
        let check_slot = |slot: &ModuleSlot, revision: usize, bytes: &[u8]| {
            assert_eq!(slot.revision(), revision as u64, "{name}");
            assert_eq!(slot.module().encode(), bytes, "{name}");
            let result = slot.run(call()).unwrap();
            assert_eq!(result, expected[revision], "{name}");
            assert!(!alias(&result, &row), "{name}");
            for (input, argument) in inputs.iter().zip(&arguments) {
                assert!(alias(input, argument), "{name}");
            }
            assert_eq!(row, list([Value::I64(3), Value::I64(5)]));
        };
        check_slot(&slot, 0, fsm);
        assert_eq!(source.matches(edit.0).count(), 1);
        let edited = Module::compile_compact(&source.replace(edit.0, edit.1)).unwrap();
        let edited_fsm = edited.encode();
        let edited_fxh = Hir::from_structured(edited).encode().unwrap();

        let refusals: Vec<_> = LEDGER
            .lines()
            .filter_map(|line| {
                if line.starts_with('#') {
                    return None;
                }
                let [id, fixture, kind, action, target, replacement, diagnostic]: [&str; 7] =
                    line.split('\t').collect::<Vec<_>>().try_into().unwrap();
                if fixture != name {
                    return None;
                }
                let wire = if kind == "fsm" { fsm } else { fxh };
                let malformed = mutate(wire, kind, action, target, replacement);
                assert_ne!(malformed, wire, "{id}");
                digest.update(u32::try_from(malformed.len()).unwrap().to_le_bytes());
                digest.update(&malformed);
                Some((id, kind, malformed, diagnostic))
            })
            .collect();
        count += refusals.len();
        for (revision, bytes) in [fsm, &edited_fsm].into_iter().enumerate() {
            if revision == 1 {
                let candidate = restored(Hir::decode(&edited_fxh).unwrap());
                assert_eq!(candidate.encode(), edited_fsm);
                assert_eq!(slot.inject(0, candidate, Some(call())).unwrap(), 1);
            }
            check_slot(&slot, revision, bytes);
            for (id, kind, wire, expected) in &refusals {
                let candidate: Result<Module, Error> = if *kind == "fsm" {
                    Module::decode(wire)
                } else {
                    Hir::decode(wire).map(restored)
                };
                assert!(
                    candidate.is_err(),
                    "{id}.{kind} decoded at revision {revision}"
                );
                let error = candidate
                    .and_then(|module| slot.inject(revision as u64, module, Some(call())))
                    .unwrap_err();
                assert_eq!(
                    error.to_string(),
                    *expected,
                    "{id}.{kind} at revision {revision}"
                );
                check_slot(&slot, revision, bytes);
            }
        }
        assert_eq!(original.run(call()).unwrap(), expected[0], "{name}");
    }
    assert_eq!(format!("{:x}", digest.finalize()), WIRE_SHA256);
    count
}
