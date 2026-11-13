//! Independent expected behavior for structured FN1 lowering.
use std::sync::atomic::{AtomicUsize, Ordering};
type Callback = fn() -> usize;
fn apply(value: i32, operation: impl FnOnce(i32) -> i32) -> i32 { operation(value) }
fn evaluate_future<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop { fn wake(self: std::sync::Arc<Self>) {} }
    let waker = Waker::from(std::sync::Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) { Poll::Ready(value) => value, Poll::Pending => panic!("unexpected pending fixture") }
}
fn touch(log: &mut Vec<i32>, value: i32) -> i32 { log.push(value); value }
fn target(log: &mut Vec<i32>) -> &mut i32 { log.push(2); log.first_mut().unwrap() }
struct Record { number: i32 }

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN arithmetic
fn arithmetic(a: i32, b: i32) -> i32 { ((a + b) * 3 - (a / b)) % 7 }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END arithmetic
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN boolean
fn boolean(a: i32, b: i32) -> bool { (a != b) && ((a < b) || (a >= b)) && !(a == b) && (a <= b || a > b) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END boolean
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN bits
fn bits(a: u32, b: u32) -> u32 { ((a << 2) | (b >> 1)) ^ (a & b) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END bits
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN tuple_array_index
fn tuple_array_index() -> (i32, usize) { let values = [2, 7, 11]; (values[1], values.len()) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END tuple_array_index
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN field_and_references
fn field_and_references() -> i32 { let mut record = Record { number: 3 }; let number = &mut record.number; *number = *number + 4; *(&record.number) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END field_and_references
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN native_closures
fn native_closures() -> i32 { let offset = 7; apply(5, move |x| x + offset) + apply(3, |x| x * 2) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END native_closures
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN closure_cast
fn closure_cast() -> Callback { (move || 42usize) as Callback }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END closure_cast
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN flow
fn flow() -> i32 { let mut total: i32 = 0; for i in 0..=5 { if i == 2 { continue; } else { } if i == 5 { break; } else { } total = total + i; } while total < 10 { total = total + 1; } return total; }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END flow
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN block_if_negation
fn block_if_negation(flag: bool) -> i32 { let result: i32 = { let temp = -4; if flag { temp } else { -temp } }; result }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END block_if_negation
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN result_propagation
fn result_propagation(value: &str) -> Result<i32, std::num::ParseIntError> { let number: i32 = value.parse()?; Ok(number + 1) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END result_propagation
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN async_await
async fn async_await() -> usize { let value = async { 41usize }; (value.await) + 1 }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END async_await
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN async_move
fn async_move() -> impl std::future::Future<Output=usize> { let value = 7usize; async move { value + 1 } }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END async_move
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN unsafe_pointer
fn unsafe_pointer() -> i32 { let value = 19i32; let pointer = &value as *const i32; unsafe { *pointer } }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END unsafe_pointer
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN call_evaluation_order
fn call_evaluation_order(log: &mut Vec<i32>) -> i32 { std::cmp::max(touch(log, 1), touch(log, 2)) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END call_evaluation_order
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN assignment_order
fn assignment_order(log: &mut Vec<i32>) { *target(log) = touch(log, 1); }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END assignment_order
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN short_circuit
fn short_circuit(counter: &AtomicUsize) -> bool { false && counter.fetch_add(1, Ordering::SeqCst) == 0 || true }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END short_circuit
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN strings_and_macro
fn strings_and_macro() -> String { format!("{}:{}", "héllo\n\t\0\"\\🌍", 42) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END strings_and_macro
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN units_and_one_tuple
fn units_and_one_tuple() -> ((),) { ((),) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END units_and_one_tuple

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN public_value
pub fn public_value() -> i32 { 23 }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END public_value
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN public_async
pub async fn public_async() -> i32 { 24 }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END public_async

#[test]
fn arithmetic_relations_bits_and_aggregates() {
    assert_eq!(arithmetic(12, 3), 6);
    assert!(boolean(3, 9)); assert!(!boolean(3, 3));
    assert_eq!(bits(9, 6), 39);
    assert_eq!(tuple_array_index(), (7, 3));
    assert_eq!(field_and_references(), 7);
    assert_eq!(units_and_one_tuple(), ((),));
    assert_eq!(public_value(), 23);
    assert_eq!(evaluate_future(public_async()), 24);
}
#[test]
fn functions_closures_cast_flow_and_blocks() {
    assert_eq!(native_closures(), 18);
    assert_eq!(closure_cast()(), 42);
    assert_eq!(flow(), 10);
    assert_eq!(block_if_negation(true), -4);
    assert_eq!(block_if_negation(false), 4);
}
#[test]
fn errors_async_and_unsafe_native_boundary() {
    assert_eq!(result_propagation("41").unwrap(), 42);
    assert!(result_propagation("no").is_err());
    assert_eq!(evaluate_future(async_await()), 42);
    assert_eq!(evaluate_future(async_move()), 8);
    assert_eq!(unsafe_pointer(), 19);
}
#[test]
fn side_effects_and_short_circuit() {
    let mut log = Vec::new();
    assert_eq!(call_evaluation_order(&mut log), 2); assert_eq!(log, vec![1,2]);
    log.clear(); assignment_order(&mut log); assert_eq!(log, vec![1,2]);
    let counter = AtomicUsize::new(0); assert!(short_circuit(&counter)); assert_eq!(counter.load(Ordering::SeqCst), 0);
}
#[test]
fn unicode_and_escapes_are_exact() { assert_eq!(strings_and_macro(), "héllo\n\t\0\"\\🌍:42"); }
