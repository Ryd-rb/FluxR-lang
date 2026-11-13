pub fn calc(mut x: i64, n: i64, s: i64) -> Option<i64> {
    while x < n {
        x = x.checked_add(s)?;
    }
    Some(x)
}
