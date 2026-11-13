use std::cell::Cell;
use std::sync::Arc;
fn main() {
    let local = Arc::new(Cell::new(0));
    std::thread::spawn(move || local.set(1)).join().unwrap();
}
