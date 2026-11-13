pub fn calc(n: i64, mut x: i64) -> Option<i64> {
    for _ in 0..n {
        x = x.checked_add(1)?;
    }
    Some(x)
}
