use fluxr::structured::Module;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "fluxr-input-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn structured_ingress_detects_fsm_and_retains_the_larger_canonical_budget() {
    let dir = Directory::new();
    let module = Module::compile_compact("v1 (pub calc () i64 () (ret 7))").unwrap();
    let mut encoded = module.encode();
    encoded.resize(256 * 1024 + 100, b' ');
    fs::write(dir.0.join("module.fsm"), &encoded).unwrap();
    fs::write(dir.0.join("legacy.json"), &encoded[6..]).unwrap();
    for (source, output) in [
        ("module.fsm", "from-fsm.flux"),
        ("legacy.json", "from-json.flux"),
    ] {
        let result = dir.cli(&["migrate-structured", source, output]);
        assert!(result.status.success(), "{result:?}");
        assert_eq!(
            Module::compile_compact(&fs::read_to_string(dir.0.join(output)).unwrap())
                .unwrap()
                .encode(),
            module.encode()
        );
    }
    let emitted = dir.cli(&["emit-native-structured", "module.fsm", "calc", "native.rs"]);
    assert!(emitted.status.success(), "{emitted:?}");
}

#[test]
fn oversized_source_and_replay_fail_before_writing_outputs() {
    let dir = Directory::new();
    let oversized = fs::File::create(dir.0.join("huge.flux")).unwrap();
    oversized.set_len(4 * 1024 * 1024).unwrap();
    for args in [
        vec!["encode", "huge.flux", "result.fxr"],
        vec!["encode-structured", "huge.flux", "result.fsm"],
        vec!["emit-native-structured", "huge.flux", "calc", "native.rs"],
    ] {
        let result = dir.cli(&args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("program exceeds byte budget"));
        assert!(!dir.0.join(args.last().unwrap()).exists());
    }
    fs::write(dir.0.join("rule.flux"), "r4 1 i0").unwrap();
    let replay = dir.cli(&["test", "rule.flux", "huge.flux"]);
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("program exceeds byte budget"));
}

#[test]
fn overlong_fsm_and_hir_keep_their_format_specific_limits() {
    let dir = Directory::new();
    let mut fsm = b"FSM\0\x01\0".to_vec();
    fsm.resize(1024 * 1024 + 7, b' ');
    fs::write(dir.0.join("too-large.fsm"), fsm).unwrap();
    let result = dir.cli(&["migrate-structured", "too-large.fsm", "result.flux"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("structured program exceeds canonical byte budget")
    );
    let hir = fs::File::create(dir.0.join("too-large.fxh")).unwrap();
    hir.set_len(512 * 1024 + 1).unwrap();
    let result = dir.cli(&["restore-hir", "too-large.fxh", "restored.fsm"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("FXH input exceeds byte limit"));
    assert!(!dir.0.join("result.flux").exists());
    assert!(!dir.0.join("restored.fsm").exists());
}
