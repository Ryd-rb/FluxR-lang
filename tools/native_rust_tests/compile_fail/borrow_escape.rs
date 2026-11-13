fn dangling() -> &'static str {
    let value = String::from("temporary");
    &value
}
fn main() { println!("{}", dangling()); }
