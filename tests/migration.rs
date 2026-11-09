use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

#[test]
fn migration_replays_and_binds_exact_bytes_without_overwriting() {
    let dir = std::env::temp_dir().join(format!("fluxr-migration-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let dir = TestDir(dir);
    let cases = [
        (
            "timeline",
            "well.flux",
            include_bytes!("../web/well.fluxr").as_slice(),
            "[[41,1,0],[41,1,4200],[41,1,4294967295],[0,0,0]]",
            "LSP",
            "encode",
            4,
        ),
        (
            "rule",
            "guard.flux",
            b"r4 2 # guard before division\ni0 guard 120 i1 /",
            "[[0,0],[1,0],[1,3],[]]",
            "FXD",
            "encode",
            4,
        ),
        (
            "structured",
            "counter.fluxm",
            include_bytes!("fixtures/structured_counter.fluxm").as_slice(),
            r#"[
              {"export":"tick","inputs":[{"i64":7}],"arguments":[{"record":[{"i64":0}]},{"i64":3}]},
              {"export":"tick","inputs":[{"i64":7}],"arguments":[{"record":[{"i64":0}]},{"i64":3}],
               "limits":{"instructions":0,"cells":0,"frames":0}},
              {"export":"absent","inputs":[],"arguments":[]}
            ]"#,
            "FSM",
            "encode-structured",
            3,
        ),
    ];
    for (kind, name, input, replay, artifact, encode, count) in cases {
        fs::write(dir.0.join(name), input).unwrap();
        fs::write(dir.0.join("replay.json"), replay).unwrap();
        let migrate = format!("migrate-{kind}");
        let run = |output| {
            cli(
                &dir.0,
                &[
                    "--receipt",
                    "--replay",
                    "replay.json",
                    &migrate,
                    name,
                    output,
                ],
            )
        };
        let first = run("first.flux");
        let second = run("second.flux");
        assert!(first.status.success(), "{:?}", first);
        assert!(second.status.success(), "{:?}", second);
        assert_eq!(first.stdout, second.stdout);
        let receipt: Value = serde_json::from_slice(&first.stdout).unwrap();
        let source = fs::read(dir.0.join("first.flux")).unwrap();
        assert_eq!(receipt["version"], 1);
        assert_eq!(receipt["artifact"], artifact);
        assert_eq!(receipt["input_sha256"], digest(input));
        assert_eq!(receipt["source_sha256"], digest(&source));
        assert_eq!(receipt["replay"]["sha256"], digest(replay.as_bytes()));
        assert_eq!(receipt["replay"]["cases"], count);

        if kind == "structured" {
            assert!(!cli(&dir.0, &[encode, name, "old.fxr"]).status.success());
            let legacy =
                fluxr::structured::Module::compile_legacy_json(std::str::from_utf8(input).unwrap())
                    .unwrap();
            fs::write(dir.0.join("old.fxr"), legacy.encode()).unwrap();
        } else {
            assert!(cli(&dir.0, &[encode, name, "old.fxr"]).status.success());
        }
        assert!(
            cli(&dir.0, &[encode, "first.flux", "new.fxr"])
                .status
                .success()
        );
        let wire = fs::read(dir.0.join("old.fxr")).unwrap();
        assert_eq!(wire, fs::read(dir.0.join("new.fxr")).unwrap());
        assert_eq!(receipt["canonical_sha256"], digest(&wire));

        let from_wire = cli(&dir.0, &[&migrate, "old.fxr", "from-wire.flux"]);
        assert!(from_wire.status.success(), "{:?}", from_wire);
        assert!(from_wire.stdout.is_empty());
        assert_eq!(source, fs::read(dir.0.join("from-wire.flux")).unwrap());

        let refused = run("first.flux");
        assert!(!refused.status.success());
        assert!(refused.stdout.is_empty());
        assert_eq!(source, fs::read(dir.0.join("first.flux")).unwrap());
        let in_place = run(name);
        assert!(!in_place.status.success());
        assert_eq!(input, fs::read(dir.0.join(name)).unwrap());

        fs::write(dir.0.join("replay.json"), "[]").unwrap();
        let refused = run("refused.flux");
        assert!(!refused.status.success());
        assert!(refused.stdout.is_empty());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("replay case count"));
        assert!(!dir.0.join("refused.flux").exists());
        for file in ["first.flux", "second.flux", "from-wire.flux"] {
            fs::remove_file(dir.0.join(file)).unwrap();
        }
    }
}

fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct TestDir(std::path::PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
