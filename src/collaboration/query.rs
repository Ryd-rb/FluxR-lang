//! Read-only dependency queries for the admitted structured language.
//!
//! These queries expose compiler-resolved call edges. They do not certify a
//! remote worker, a test oracle, host state, or dependency coverage outside the
//! structured module. A caller must bind those inputs before reusing evidence.

use super::*;

/// A declaration key independent of its source-order slot. Receiver identity is
/// separate from the spelling so methods cannot collide with free functions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationKey {
    pub receiver: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationDependencies {
    pub declaration: DeclarationKey,
    /// Stable authoring target when registered; helpers remain visible as nodes.
    pub target: Option<String>,
    pub revision: Revision,
    /// Direct resolved callees, sorted by stable declaration key.
    pub callees: Vec<DeclarationKey>,
}

/// A complete call graph for one admitted structured snapshot. It deliberately
/// contains unregistered helpers and recursive edges. The environment revision
/// conservatively includes all input, record and enum declarations, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyManifest {
    pub schema: String,
    pub snapshot: Revision,
    pub environment: Revision,
    pub declarations: Vec<DeclarationDependencies>,
}

/// A conservative comparison of two immutable snapshots. The report is useful
/// for scheduling relevant checks, but is not permission to skip required gates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorImpact {
    pub schema: String,
    pub before: Revision,
    pub after: Revision,
    pub environment_changed: bool,
    /// Includes changed, added and removed declaration keys.
    pub changed_declarations: Vec<DeclarationKey>,
    /// Shared target IDs whose immutable edit contract changed.
    pub changed_contracts: Vec<String>,
    /// Current target IDs with changed behavior or edit contracts, including
    /// new targets. A mode-only change can affect admission without code edits.
    pub affected_targets: Vec<String>,
    pub added_targets: Vec<String>,
    pub removed_targets: Vec<String>,
}

impl Snapshot {
    fn declaration_key(&self, index: usize) -> DeclarationKey {
        let function = &self.source.functions[index];
        DeclarationKey {
            receiver: function.receiver.clone(),
            name: function.name.clone(),
        }
    }

    fn environment_revision(&self) -> Result<Revision, Error> {
        Ok(self.environment)
    }

    /// Export compiler-derived dependencies without rerunning source resolution.
    /// This graph has cycles when source calls are recursive. Consumers must use
    /// graph traversal or SCC condensation, never assume it is a task DAG.
    pub fn dependency_manifest(&self) -> Result<DependencyManifest, Error> {
        let registered: BTreeMap<_, _> = self
            .targets
            .iter()
            .map(|(target, &(index, _))| (index, target.clone()))
            .collect();
        let mut declarations = self
            .source
            .functions
            .iter()
            .enumerate()
            .map(|(index, _function)| {
                let mut callees: Vec<_> = self.dependencies[index]
                    .iter()
                    .map(|&callee| self.declaration_key(callee))
                    .collect();
                callees.sort();
                Ok(DeclarationDependencies {
                    declaration: self.declaration_key(index),
                    target: registered.get(&index).cloned(),
                    // This cached digest uses the unchanged v1 serialization;
                    // unregistered helpers can exceed a proposal's byte budget.
                    revision: self.declaration_revisions[index],
                    callees,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        declarations.sort_by(|a, b| a.declaration.cmp(&b.declaration));
        Ok(DependencyManifest {
            schema: "fluxr-structured-dependencies-v1".into(),
            snapshot: self.id,
            environment: self.environment_revision()?,
            declarations,
        })
    }

    /// Find registered roots transitively dependent on proposed target writes.
    /// Unknown targets are errors rather than an empty impact set. All roots in
    /// a recursive component and all their callers are returned.
    pub fn affected_targets<'a>(
        &self,
        targets: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeSet<String>, Error> {
        let changed = targets
            .into_iter()
            .map(|target| {
                self.targets
                    .get(target)
                    .map(|&(index, _)| index)
                    .ok_or_else(|| Error(format!("unknown impact target {target}")))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let affected = self.reverse_closure(&changed);
        Ok(affected
            .into_iter()
            .filter_map(|index| self.target_keys[index].clone())
            .collect())
    }

    pub(super) fn reverse_closure(&self, changed: &BTreeSet<usize>) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut pending: Vec<_> = changed.iter().copied().collect();
        while let Some(index) = pending.pop() {
            if seen.insert(index) {
                pending.extend(self.reverse_dependencies[index].iter().copied());
            }
        }
        seen
    }

    /// Compare complete behavioral identities, including helper changes and
    /// schema changes. Reordering declarations alone does not change behavior
    /// pins. Deletion and target remapping are visible explicitly.
    pub fn impact_since(&self, before: &Snapshot) -> Result<BehaviorImpact, Error> {
        let index = |snapshot: &Snapshot| {
            snapshot
                .source
                .functions
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    (
                        snapshot.declaration_key(index),
                        (index, snapshot.declaration_revisions[index]),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let old = index(before);
        let new = index(self);
        let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
        let changed_declarations: Vec<_> = keys
            .into_iter()
            .filter(|key| old.get(*key).map(|entry| entry.1) != new.get(*key).map(|entry| entry.1))
            .cloned()
            .collect();
        // Use both graphs: a removed edge or declaration may only be present in
        // the old graph. Full behavior pins are not needed to identify changed
        // closures, and this keeps previously cold behavior memos cold.
        let changed_indices = |graph: &BTreeMap<DeclarationKey, (usize, Revision)>| {
            changed_declarations
                .iter()
                .filter_map(|key| graph.get(key).map(|entry| entry.0))
                .collect()
        };
        let old_affected = before.reverse_closure(&changed_indices(&old));
        let new_affected = self.reverse_closure(&changed_indices(&new));
        let environment_changed = self.environment != before.environment;
        let mut affected_targets = Vec::new();
        let mut changed_contracts = Vec::new();
        let mut added_targets = Vec::new();
        let mut removed_targets = Vec::new();
        for target in self.targets.keys() {
            if !before.targets.contains_key(target) {
                added_targets.push(target.clone());
                affected_targets.push(target.clone());
            } else {
                let contract_changed = self.contracts[target] != before.contracts[target];
                if contract_changed {
                    changed_contracts.push(target.clone());
                }
                if contract_changed
                    || environment_changed
                    || new_affected.contains(&self.targets[target].0)
                    || old_affected.contains(&before.targets[target].0)
                {
                    affected_targets.push(target.clone());
                }
            }
        }
        for target in before.targets.keys() {
            if !self.targets.contains_key(target) {
                removed_targets.push(target.clone());
            }
        }
        Ok(BehaviorImpact {
            schema: "fluxr-structured-impact-v1".into(),
            before: before.id,
            after: self.id,
            environment_changed,
            changed_declarations,
            changed_contracts,
            affected_targets,
            added_targets,
            removed_targets,
        })
    }
}

#[cfg(test)]
mod memo_tests {
    use super::*;

    fn base() -> Snapshot {
        Snapshot::new(
            Source::parse_compact(
                "v1 (fn leaf () i64 () (ret 1))
                (pub caller () i64 () (ret (call leaf)))
                (pub unrelated () i64 () (ret 0))",
            )
            .unwrap(),
            ["leaf", "caller", "unrelated"]
                .map(|name| Target {
                    id: name.into(),
                    function: name.into(),
                    receiver: None,
                    mode: Mode::Exclusive,
                })
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn impact_queries_do_not_initialize_unrequested_behavior_memos() {
        let base = base();
        let candidate = base
            .prepare(&[Change {
                author: "query-locality".into(),
                requires: BTreeMap::new(),
                edits: vec![Edit::ReplaceBody {
                    target: "leaf".into(),
                    expected: base.edit_revision("leaf").unwrap(),
                    body: vec![Stmt::Return {
                        value: Expr::I64 { value: 9 },
                    }],
                }],
            }])
            .unwrap();
        let next = candidate.snapshot();
        assert_eq!(
            next.impact_since(&base).unwrap().affected_targets,
            vec!["caller", "leaf"]
        );
        for snapshot in [&base, next] {
            assert!(
                snapshot
                    .behavior_cache
                    .iter()
                    .all(|memo| memo.get().is_none())
            );
        }
    }

    #[test]
    fn only_unchanged_closures_share_memos_across_generations() {
        let base = base();
        for name in ["leaf", "caller", "unrelated"] {
            base.behavior_revision(name).unwrap();
        }
        let result = base
            .prepare(&[Change {
                author: "agent".into(),
                requires: BTreeMap::new(),
                edits: vec![Edit::ReplaceBody {
                    target: "leaf".into(),
                    expected: base.edit_revision("leaf").unwrap(),
                    body: vec![Stmt::Return {
                        value: Expr::I64 { value: 2 },
                    }],
                }],
            }])
            .unwrap();
        let next = result.snapshot();
        for index in [0, 1] {
            assert!(!Arc::ptr_eq(
                &base.behavior_cache[index],
                &next.behavior_cache[index]
            ));
            assert!(next.behavior_cache[index].get().is_none());
        }
        assert!(Arc::ptr_eq(
            &base.behavior_cache[2],
            &next.behavior_cache[2]
        ));
        assert!(next.behavior_cache[2].get().is_some());
        for name in ["leaf", "caller", "unrelated"] {
            let index = next.targets[name].0;
            assert_eq!(
                next.behavior_revision(name).unwrap(),
                Some(next.compute_behavior_revision(index).unwrap())
            );
        }
    }
}
