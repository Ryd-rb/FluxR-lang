//! Trusted Rust/libtest capabilities inside an emitted FluxR application.
//! Every checked call installs its own runtime: the generated admission test
//! is independent and need not run before any user test.

fn main() {
    panic!("the native main must not execute under the test harness");
}

#[test]
fn checked_runtime_isolated() {
    use fluxr::runtime::{Limits, Runtime, Value};
    let runtime = Runtime::new();
    runtime
        .install("case", None, "fluxr 1\npub answer 0 int ret 42\n")
        .unwrap();
    let result = runtime
        .call("case", "answer", &[], &[], Limits::default())
        .unwrap();
    assert_eq!(result.value, Value::I64(42));
    println!("NATIVE_CHECKED_TEST_EXECUTED");
}

#[test]
fn result_success() -> Result<(), String> {
    if 21_i64.checked_mul(2) == Some(42) {
        Ok(())
    } else {
        Err("checked Rust arithmetic did not match".into())
    }
}

#[test]
#[should_panic(expected = "expected native panic")]
fn expected_panic() {
    panic!("expected native panic");
}

#[test]
#[ignore = "explicit include-ignored coverage"]
fn ignored_success() {
    println!("NATIVE_IGNORED_TEST_EXECUTED");
    assert_eq!(42, 40 + 2);
}

#[test]
#[ignore = "intentional failing test verifies nonzero libtest status"]
fn intentional_failure() {
    panic!("EXPECTED_NATIVE_TEST_FAILURE");
}
