pub fn run(n: i64) -> Option<i64> {
    let mut i = 0_i64;
    let mut total = 0_i64;
    while i < n {
        total = total.checked_add(i)?;
        i = i.checked_add(1)?;
    }
    Some(total)
}
