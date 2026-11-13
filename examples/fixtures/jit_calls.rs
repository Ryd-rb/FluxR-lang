fn square(x: i64) -> Option<i64> {
    x.checked_mul(x)?.checked_add(3)
}
pub fn run(x: i64, y: i64) -> Option<i64> {
    square(x)?.checked_add(square(y)?)
}
