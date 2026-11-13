use fluxr::collaboration::{
    Change, Edit, Mode, Snapshot, Target, VersionedProgram, VersionedSnapshot,
};
use fluxr::structured::{Expr, Source, Stmt};

fn snapshot() -> Snapshot {
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

fn edit(capture: &VersionedSnapshot, value: i64) -> Change {
    Change {
        author: "agent".into(),
        requires: Default::default(),
        edits: vec![Edit::ReplaceBody {
            target: "value".into(),
            expected: capture.snapshot().edit_revision("value").unwrap(),
            body: vec![Stmt::Return {
                value: Expr::I64 { value },
            }],
        }],
    }
}

#[test]
fn returning_to_identical_code_does_not_reauthorize_old_work() {
    let base = snapshot();
    let mut program = VersionedProgram::new(base.clone());
    let original = program.capture();
    let stale = original.prepare(&[edit(&original, 9)]).unwrap();
    let first = original.prepare(&[edit(&original, 1)]).unwrap();
    let first_receipt = program.activate(first, 10).unwrap();
    assert_eq!(first_receipt.after.epoch, 1);
    let later = program.capture();
    let restore = later.prepare(&[edit(&later, 0)]).unwrap();
    program.activate(restore, 11).unwrap();
    assert_eq!(program.snapshot().id(), base.id());
    assert_eq!(program.version().epoch, 2);
    assert!(program.activate(stale, 12).is_err());
    assert_eq!(program.version().epoch, 2);
    assert_eq!(original.version().epoch, 0);
    assert_eq!(original.snapshot().id(), base.id());
}

#[test]
fn another_instance_cannot_publish_a_candidate_with_identical_version() {
    let base = snapshot();
    let left = VersionedProgram::new(base.clone());
    let mut right = VersionedProgram::new(base);
    assert_eq!(left.version(), right.version());
    let capture = left.capture();
    let candidate = capture.prepare(&[edit(&capture, 1)]).unwrap();
    assert!(
        right
            .activate(candidate, 0)
            .unwrap_err()
            .to_string()
            .contains("another program instance")
    );
    assert_eq!(right.version().epoch, 0);
}

#[test]
fn unchanged_publication_fences_older_candidates_and_preserves_checked_rejections() {
    let mut program = VersionedProgram::new(snapshot());
    let capture = program.capture();
    let stale = capture.prepare_checked(&[]).unwrap();
    let no_op = capture.prepare(&[]).unwrap();
    let before = program.version();
    let receipt = program.activate(no_op, 0).unwrap();
    assert_eq!(receipt.before.snapshot, receipt.after.snapshot);
    assert_eq!(receipt.after.epoch, before.epoch + 1);
    assert!(program.activate(stale, 1).is_err());
}

#[test]
fn independent_workers_prepare_without_borrowing_the_publisher() {
    let mut program = VersionedProgram::new(snapshot());
    let captures = [program.capture(), program.capture()];
    let candidates = std::thread::scope(|scope| {
        captures
            .iter()
            .enumerate()
            .map(|(index, capture)| {
                scope.spawn(move || capture.prepare(&[edit(capture, index as i64 + 1)]).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut iter = candidates.into_iter();
    let first = iter.next().unwrap();
    assert_eq!(first.candidate().accepted.len(), 1);
    assert!(first.candidate().rejected.is_empty());
    program.activate(first, 0).unwrap();
    assert!(program.activate(iter.next().unwrap(), 1).is_err());
}
