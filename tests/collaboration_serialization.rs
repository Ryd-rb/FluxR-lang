use fluxr::collaboration::{
    Change, CheckedChange, Edit, Mode, Rejection, Revision, Snapshot, Target,
};
use fluxr::structured::{Expr, Source, Stmt};
use sha2::{Digest, Sha256};

fn base() -> Snapshot {
    Snapshot::new(
        Source::parse_compact("v1 (pub value () i64 () (ret 0))").unwrap(),
        vec![Target {
            id: "value".into(),
            function: "value".into(),
            receiver: None,
            mode: Mode::Exclusive,
        }],
    )
    .unwrap()
}

fn change(base: &Snapshot, author: String) -> Change {
    Change {
        author,
        requires: Default::default(),
        edits: vec![Edit::ReplaceBody {
            target: "value".into(),
            expected: base.edit_revision("value").unwrap(),
            body: vec![Stmt::Return {
                value: Expr::I64 { value: 1 },
            }],
        }],
    }
}

fn reference_hash(value: &impl serde::Serialize) -> Revision {
    let digest = Sha256::digest(serde_json::to_vec(value).unwrap());
    format!("{digest:x}").parse().unwrap()
}

#[test]
fn streaming_legacy_and_checked_ids_preserve_their_original_serializations() {
    let base = base();
    let proposal = change(&base, "unicode author \u{1f916} \"quoted\"".into());
    let reference = reference_hash(&proposal);
    let result = base.prepare(std::slice::from_ref(&proposal)).unwrap();
    assert_eq!(result.accepted, vec![reference]);
    let checked = CheckedChange {
        change: proposal,
        behavior_requires: base.behavior_read_set(["value"]).unwrap(),
    };
    let reference = reference_hash(&("fluxr-checked-change-v1", &checked));
    let result = base.prepare_checked(&[checked]).unwrap();
    assert_eq!(result.accepted, vec![reference]);
}

#[test]
fn oversized_rejections_retain_exact_ids_and_do_not_block_independent_valid_work() {
    let base = base();
    let proposal = change(&base, "x".repeat(1024 * 1024));
    let reference = reference_hash(&proposal);
    let result = base
        .prepare(&[proposal.clone(), change(&base, "valid".into())])
        .unwrap();
    assert_eq!(result.accepted.len(), 1);
    assert!(matches!(
        result.rejected.get(&reference),
        Some(Rejection::Invalid(_))
    ));
    let checked = CheckedChange {
        change: proposal,
        behavior_requires: Default::default(),
    };
    let reference = reference_hash(&("fluxr-checked-change-v1", &checked));
    let result = base
        .prepare_checked(&[
            checked,
            CheckedChange {
                change: change(&base, "valid".into()),
                behavior_requires: Default::default(),
            },
        ])
        .unwrap();
    assert_eq!(result.accepted.len(), 1);
    assert!(matches!(
        result.rejected.get(&reference),
        Some(Rejection::Invalid(_))
    ));
}
