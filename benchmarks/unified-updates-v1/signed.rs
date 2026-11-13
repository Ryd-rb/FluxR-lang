pub fn run(mut x: i64, y: i64) -> Option<i64> {
    x = x.checked_add(y)?;
    x = x.checked_mul(3)?;
    Some(x)
}
