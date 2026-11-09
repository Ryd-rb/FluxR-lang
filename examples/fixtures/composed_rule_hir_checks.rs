use fluxr::{
    RuleProgram,
    hir::{Artifact, Hir},
    structured::{Limits, Value, Workspace},
};

fn word(text: &str) -> i64 {
    match text.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}

pub fn check() -> usize {
    const LEDGER: &str = include_str!("../../tests/fixtures/composed-rule/ledger.txt");
    const LEGACY_TRAP: &str = "rule arithmetic overflow or division by zero";
    const CHECKED_TRAP: &str = "invalid or overflowing integer arithmetic";

    let mut rows = 0;
    for (case, source, fxd, fxh, entry_cells) in [
        (
            "signed",
            include_str!("../../tests/fixtures/composed-rule/signed.flux"),
            include_bytes!("../../tests/fixtures/composed-rule/signed.fxr").as_slice(),
            include_bytes!("../../tests/fixtures/composed-rule/signed.fxh").as_slice(),
            19,
        ),
        (
            "mixed",
            include_str!("../../tests/fixtures/composed-rule/mixed.flux"),
            include_bytes!("../../tests/fixtures/composed-rule/mixed.fxr").as_slice(),
            include_bytes!("../../tests/fixtures/composed-rule/mixed.fxh").as_slice(),
            21,
        ),
    ] {
        let compiled = RuleProgram::compile_text(source).unwrap();
        let archived = RuleProgram::decode(fxd).unwrap();
        let from_fxd = Hir::from_rule(archived.clone()).unwrap();
        let from_fxh = Hir::decode(fxh).unwrap();
        assert_eq!(compiled.encode(), fxd);
        assert_eq!(from_fxd.encode().unwrap(), fxh);
        assert_eq!(from_fxh.encode().unwrap(), fxh);
        if case == "mixed" {
            assert_eq!(
                from_fxh.source().unwrap_err().to_string(),
                "structured projection lacks rule raw binary32 transport"
            );
        }
        let projections = [
            from_fxd.prepare_rule_projection().unwrap(),
            from_fxh.prepare_rule_projection().unwrap(),
        ];
        let Artifact::Rule(restored) = from_fxh.into_artifact() else {
            panic!("FXH lost its rule origin");
        };
        assert_eq!(restored.encode(), fxd);
        let mut workspace = Workspace::default();

        for row in LEDGER.lines().filter_map(|line| {
            let (name, row) = line.split_once(' ')?;
            (name == case).then_some(row)
        }) {
            let mut fields = row.split_ascii_whitespace();
            let id = fields.next().unwrap();
            let instructions = fields.next().unwrap().parse().unwrap();
            let expected = match fields.next().unwrap() {
                "div" | "add" | "rem" => Err(CHECKED_TRAP),
                value => Ok(word(value)),
            };
            let arguments: Vec<_> = fields.map(word).collect();
            for legacy in [&compiled, &archived, &restored] {
                assert_eq!(
                    legacy.evaluate(&arguments).map_err(|err| err.to_string()),
                    expected.map_err(|_| LEGACY_TRAP.to_owned()),
                    "{case} {id}"
                );
            }

            let arguments: Vec<_> = arguments.into_iter().map(Value::I64).collect();
            // Entry charges the arguments, their slots, four stack slots and
            // the frame. Only a successful return charges the result cell.
            let cells = entry_cells + usize::from(expected.is_ok());
            let exact = Limits {
                instructions,
                cells,
                frames: 1,
            };
            for projection in &projections {
                for (limits, outcome) in [
                    (exact, expected.map(Value::I64)),
                    (
                        Limits {
                            instructions: instructions - 1,
                            ..exact
                        },
                        Err("module instruction budget exhausted"),
                    ),
                    (
                        Limits {
                            cells: cells - 1,
                            ..exact
                        },
                        Err("module cell budget exhausted"),
                    ),
                    (
                        Limits { frames: 0, ..exact },
                        Err("module frame limit exceeded"),
                    ),
                ] {
                    assert_eq!(
                        projection
                            .run_with(&arguments, limits, &mut workspace)
                            .map_err(|err| err.to_string()),
                        outcome.map_err(str::to_owned),
                        "{case} {id} at {limits:?}"
                    );
                }
            }
            rows += 1;
        }
    }
    rows
}
