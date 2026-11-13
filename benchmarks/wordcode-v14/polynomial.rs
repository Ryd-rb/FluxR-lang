pub fn calc(x: i64, y: i64, z: i64) -> Option<i64> {
    x.checked_mul(x)?
        .checked_add(y.checked_mul(2)?)?
        .checked_add(z)
}
