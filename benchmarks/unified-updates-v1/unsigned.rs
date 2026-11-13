pub fn run(mut x: u64, y: u64) -> Option<u64> {
    x = x.checked_add(y)?;
    x = x.checked_mul(3)?;
    Some(x)
}
