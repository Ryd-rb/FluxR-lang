use std::rc::Rc;
fn main() {
    let local = Rc::new(42);
    std::thread::spawn(move || drop(local)).join().unwrap();
}
