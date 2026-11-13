pub fn run(mut x: u64, y: u64) -> Option<u64> {
    x = x.checked_sub(y)?;
    x = x.checked_div(3)?;
    x = x.checked_rem(5)?;
    Some(x)
}
