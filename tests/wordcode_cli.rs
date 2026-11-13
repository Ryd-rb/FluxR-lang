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
            "fluxr-wordcode-cli-{}-{}",
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
fn explicit_wordcode_cli_emits_ordinary_fsm_and_lossless_fa1() {
    let dir = Directory::new();
    let source = include_str!("../benchmarks/wordcode-v14/affine.wk1");
    let reference =
        Module::compile_agent(include_str!("../benchmarks/wordcode-v14/affine.fa1")).unwrap();
    fs::write(dir.0.join("program.wk1"), source).unwrap();
    for (command, output) in [
        ("encode-wordcode", "program.fsm"),
        ("expand-wordcode", "program.fa1"),
    ] {
        let result = dir.cli(&[command, "program.wk1", output]);
        assert!(result.status.success(), "{result:?}");
    }
    assert_eq!(
        fs::read(dir.0.join("program.fsm")).unwrap(),
        reference.encode()
    );
    let expanded = fs::read_to_string(dir.0.join("program.fa1")).unwrap();
    assert!(expanded.starts_with("fa1 v2"));
    assert_eq!(
        Module::compile_agent(&expanded).unwrap().encode(),
        reference.encode()
    );
    let old_reader = dir.cli(&["encode-agent", "program.wk1", "wrong.fsm"]);
    assert!(!old_reader.status.success());
    assert!(!dir.0.join("wrong.fsm").exists());
}

#[test]
fn wordcode_cli_refusals_preserve_outputs_and_bound_ingress() {
    let dir = Directory::new();
    for source in [
        "wk1 stale v2 pub calc 0 int ret 1",
        "wk1 core v2 pub calc 0 int ret square true",
        "wk1 core v2 pub calc 0 int ret square add 1 2",
        "wk1 core v2 pub calc 0 int ret",
        "fa1 v2 pub calc 0 int ret 1",
    ] {
        fs::write(dir.0.join("invalid.wk1"), source).unwrap();
        for command in ["encode-wordcode", "expand-wordcode"] {
            fs::write(dir.0.join("existing"), b"retained-before-refusal").unwrap();
            let result = dir.cli(&[command, "invalid.wk1", "existing"]);
            assert!(!result.status.success());
            assert_eq!(
                fs::read(dir.0.join("existing")).unwrap(),
                b"retained-before-refusal"
            );
            let absent = dir.cli(&[command, "invalid.wk1", "absent"]);
            assert!(!absent.status.success());
            assert!(!dir.0.join("absent").exists());
        }
    }
    let oversized = fs::File::create(dir.0.join("huge.wk1")).unwrap();
    oversized.set_len(4 * 1024 * 1024).unwrap();
    for command in ["encode-wordcode", "expand-wordcode"] {
        let result = dir.cli(&[command, "huge.wk1", "huge-output"]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("program exceeds byte budget"));
        assert!(!dir.0.join("huge-output").exists());
    }
}
