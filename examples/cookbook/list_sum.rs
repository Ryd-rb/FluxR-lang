pub fn run(values: &[i64]) -> Option<i64> {
    let mut total = 0_i64;
    for &value in values.iter().rev() {
        total = total.checked_add(value)?;
    }
    Some(total)
}
