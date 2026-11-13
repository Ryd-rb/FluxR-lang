pub fn run(values: &[i64]) -> Option<i64> {
    let mut count = 0_i64;
    for _ in values {
        count = count.checked_add(1)?;
    }
    Some(count)
}
