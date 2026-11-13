pub fn run(x: i64, factor: i64, offset: i64) -> Option<i64> {
    x.checked_mul(factor)?.checked_add(offset)
}
