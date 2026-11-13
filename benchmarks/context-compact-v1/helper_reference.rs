fn step(x: i64) -> Option<i64> {
    x.checked_add(3)
}
fn calc(x: i64) -> Option<i64> {
    step(x.checked_mul(2)?)
}
