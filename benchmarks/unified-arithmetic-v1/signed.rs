pub fn run(mut x: i64, y: i64) -> Option<i64> {
    x = x.checked_sub(y)?;
    x = x.checked_div(3)?;
    x = x.checked_rem(5)?;
    Some(x)
}
