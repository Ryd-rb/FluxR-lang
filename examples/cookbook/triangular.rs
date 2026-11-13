pub fn run(mut n: i64) -> Option<i64> {
    let mut total = 0_i64;
    while n > 0 {
        total = total.checked_add(n)?;
        n = n.checked_sub(1)?;
    }
    Some(total)
}
