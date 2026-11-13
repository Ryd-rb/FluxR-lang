fn square_plus_one(x: i64) -> Option<i64> {
    x.checked_mul(x)?.checked_add(1)
}
pub fn run(x: i64, y: i64) -> Option<i64> {
    square_plus_one(x)?.checked_add(square_plus_one(y)?)
}
