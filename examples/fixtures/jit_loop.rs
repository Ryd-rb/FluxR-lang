pub fn run(x: i64, mut n: i64) -> Option<i64> {
    let mut acc = x;
    while n > 0 {
        acc = acc.checked_add(n.checked_mul(3)?)?;
        n = n.checked_sub(1)?;
    }
    Some(acc)
}
