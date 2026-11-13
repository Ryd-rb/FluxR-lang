#![deny(invalid_atomic_ordering)]
use std::sync::atomic::{AtomicUsize, Ordering};
fn main() { let value = AtomicUsize::new(0); let _ = value.load(Ordering::Release); }
