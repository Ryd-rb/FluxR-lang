fn calc(x: i64, y: i64) -> Option<i64> {
    x.checked_mul(y)?.checked_add(17)?.checked_rem(997)
}
