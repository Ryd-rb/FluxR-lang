//! Replay a frozen ledger against the portable FSM decoder and slot admission.
use super::{GOOD, INDEXED, PROJECTED, SOURCE, check_aliases, check_result, point};
use crate::corpus_checks::structured_boundary_checks::replace_fsm_body;
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Source};
use serde::Deserialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Ledger<'a> {
    sha256: [&'a str; 2],
    malformed_sha256: [&'a str; 2],
    #[serde(borrow)]
    refusals: Vec<Refusal<'a>>,
}

#[derive(Deserialize)]
struct Refusal<'a>(
    &'a str,
    String,
    #[serde(borrow)] Vec<(&'a str, Json, usize)>,
);

pub(crate) fn check_method_artifact_safety() -> usize {
    let ledger: Ledger<'_> =
        serde_json::from_str(include_str!("method_artifact_safety.json")).unwrap();
    let old = format!("{SOURCE}\n{}", include_str!("method_edit_identity.fluxr"));
    let edited = old.replace("(+ self.value delta)", "(- self.value delta)");
    let [old_bytes, edited_bytes] = [&old, &edited].map(|source| {
        let original = Module::compile_compact(source).unwrap().encode();
        let dotted = source.replace("(m holder.point update", "(holder.point.update");
        assert_eq!(Module::compile_compact(&dotted).unwrap().encode(), original);
        original
    });
    for (bytes, digest) in [&old_bytes, &edited_bytes].into_iter().zip(ledger.sha256) {
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), digest);
        assert_eq!(Module::decode(bytes).unwrap().encode(), *bytes);
    }
    let grants = Source::parse_compact(&old).unwrap().inputs;
    let mut slot = ModuleSlot::new(Module::decode(&old_bytes).unwrap(), &grants).unwrap();
    let inputs = GOOD.inputs();
    let indexed = GOOD.args(false, &inputs[1]);
    let projected = GOOD.args(true, &inputs[1]);
    let indexed_call = || Invocation {
        export: "indexed",
        inputs: &inputs,
        arguments: &indexed,
        limits: INDEXED,
    };
    let twin = [point(7, 60)];
    for (revision, original) in [&old_bytes, &edited_bytes].into_iter().enumerate() {
        let revision = revision as u64;
        if revision == 1 {
            let candidate = Module::decode(original).unwrap();
            assert_eq!(slot.inject(0, candidate, Some(indexed_call())).unwrap(), 1);
        }
        let check_slot = |slot: &ModuleSlot| {
            assert_eq!(slot.revision(), revision);
            assert_eq!(&slot.module().encode(), original);
            for (export, args, cap, projection) in [
                ("indexed", &indexed, INDEXED, false),
                ("projected", &projected, PROJECTED, true),
            ] {
                let output = slot
                    .run(Invocation {
                        export,
                        inputs: &inputs,
                        arguments: args,
                        limits: cap,
                    })
                    .unwrap();
                assert_eq!(
                    output,
                    GOOD.expected(projection, cap, revision == 1).unwrap()
                );
                check_result(&output, &inputs[1]);
                check_aliases(GOOD, projection, &inputs, args);
            }
            assert_eq!(
                slot.run(Invocation {
                    export: "twin",
                    inputs: &inputs,
                    arguments: &twin,
                    limits: Limits::default(),
                })
                .unwrap(),
                point(13, 60)
            );
        };
        check_slot(&slot);
        let body: Json = serde_json::from_slice(&original[6..]).unwrap();
        let mut malformed_digest = Sha256::new();
        for Refusal(id, expected, patches) in &ledger.refusals {
            let mut changed = body.clone();
            for (path, replacement, copies) in patches {
                let target = changed
                    .pointer_mut(path)
                    .unwrap_or_else(|| panic!("{id}: {path}"));
                let replacement = if *copies == 1 {
                    replacement.clone()
                } else {
                    Json::Array(vec![replacement.clone(); *copies])
                };
                assert_ne!(*target, replacement, "{id}: {path}");
                *target = replacement;
            }
            let bytes = replace_fsm_body(original, &serde_json::to_vec(&changed).unwrap());
            malformed_digest.update(&bytes);
            let candidate = Module::decode(&bytes);
            assert_eq!(
                candidate.as_ref().unwrap_err().to_string(),
                *expected,
                "{id} at revision {revision}"
            );
            let refusal = candidate
                .and_then(|module| slot.inject(revision, module, Some(indexed_call())))
                .unwrap_err();
            assert_eq!(
                refusal.to_string(),
                *expected,
                "{id} at revision {revision}"
            );
            check_slot(&slot);
        }
        assert_eq!(
            format!("{:x}", malformed_digest.finalize()),
            ledger.malformed_sha256[revision as usize]
        );
    }
    ledger.refusals.len()
}
