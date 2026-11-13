pub fn calc(x: i64, y: i64, z: i64) -> Option<i64> {
    x.checked_mul(y)?.checked_add(z)
}
