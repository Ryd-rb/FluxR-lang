// No explicit FluxR dependency call: the compiler must initialize the VM.
fn main() {
    use std::io::Write;
    println!("UNIFIED_MINIMAL_READY {}", std::process::id());
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}
