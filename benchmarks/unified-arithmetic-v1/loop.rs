pub fn run(mut n: i64) -> Option<i64> {
    let mut total = 100_i64;
    while 0 < n {
        total = total.checked_sub(n)?;
        n = n.checked_sub(1)?;
    }
    Some(total)
}
