//! Opt-in local activation fencing for asynchronous authoring.
//!
//! Code identity alone is intentionally sufficient for `LiveProgram`, but a
//! host may need to distinguish two activations of the same code. This API adds
//! a monotonically increasing epoch and an opaque host-instance binding. It is
//! not a distributed lease, durable recovery protocol or state transaction.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramVersion {
    pub snapshot: Revision,
    pub epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VersionedActivation {
    pub tick: u64,
    pub before: ProgramVersion,
    pub after: ProgramVersion,
}

/// An immutable authoring capture. Cloning this handle is cheap and does not
/// lock the publisher while agents perform compilation or validation.
#[derive(Clone)]
pub struct VersionedSnapshot {
    snapshot: Snapshot,
    version: ProgramVersion,
    authority: Arc<()>,
}

impl VersionedSnapshot {
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn version(&self) -> ProgramVersion {
        self.version
    }

    pub fn prepare(&self, changes: &[Change]) -> Result<VersionedCandidate, Error> {
        self.bind(self.snapshot.prepare(changes)?)
    }

    pub fn prepare_checked(&self, changes: &[CheckedChange]) -> Result<VersionedCandidate, Error> {
        self.bind(self.snapshot.prepare_checked(changes)?)
    }

    fn bind(&self, candidate: Candidate) -> Result<VersionedCandidate, Error> {
        Ok(VersionedCandidate {
            candidate,
            base: self.version,
            authority: Arc::clone(&self.authority),
        })
    }
}

/// Only a capture can construct this value. Serialized reports are not bearer
/// capabilities and cannot be deserialized into an activatable candidate.
pub struct VersionedCandidate {
    candidate: Candidate,
    base: ProgramVersion,
    authority: Arc<()>,
}

impl VersionedCandidate {
    pub fn base(&self) -> ProgramVersion {
        self.base
    }

    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    pub fn snapshot(&self) -> &Snapshot {
        self.candidate.snapshot()
    }
}

/// A single-host publisher with a short, exclusive activation operation.
///
/// Capture under the host's publication lock, prepare and check outside it,
/// then activate under the lock. A changed epoch rejects a stale candidate,
/// even when the source has returned to an earlier snapshot identity (ABA).
/// Activation does not enforce test policy; the trusted host still owns that
/// decision and any atomic coupling to persistent or external state.
pub struct VersionedProgram {
    active: Snapshot,
    epoch: u64,
    authority: Arc<()>,
}

impl VersionedProgram {
    pub fn new(active: Snapshot) -> Self {
        Self {
            active,
            epoch: 0,
            authority: Arc::new(()),
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.active
    }

    pub fn version(&self) -> ProgramVersion {
        ProgramVersion {
            snapshot: self.active.id(),
            epoch: self.epoch,
        }
    }

    pub fn capture(&self) -> VersionedSnapshot {
        VersionedSnapshot {
            snapshot: self.active.clone(),
            version: self.version(),
            authority: Arc::clone(&self.authority),
        }
    }

    /// Every successful publication advances the epoch, including publication
    /// of an unchanged snapshot. Exhaustion fails without changing active code.
    pub fn activate(
        &mut self,
        candidate: VersionedCandidate,
        tick: u64,
    ) -> Result<VersionedActivation, Error> {
        if !Arc::ptr_eq(&self.authority, &candidate.authority) {
            return Err(Error(
                "candidate belongs to another program instance".into(),
            ));
        }
        let before = self.version();
        if candidate.base != before {
            return Err(Error(
                "candidate was prepared against an older program epoch".into(),
            ));
        }
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| Error("program epoch exhausted".into()))?;
        self.active = candidate.candidate.snapshot;
        self.epoch = epoch;
        Ok(VersionedActivation {
            tick,
            before,
            after: self.version(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_epoch_cannot_publish_or_wrap() {
        let snapshot = Snapshot::new(
            Source::parse_compact("v1 (pub value () i64 () (ret 0))").unwrap(),
            vec![],
        )
        .unwrap();
        let mut program = VersionedProgram::new(snapshot);
        program.epoch = u64::MAX;
        let before = program.version();
        let candidate = program.capture().prepare(&[]).unwrap();
        assert!(
            program
                .activate(candidate, 0)
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert_eq!(program.version(), before);
    }
}
