pub fn run(uninitialized: Option<i64>, divisor: i64) -> Result<i64, &'static str> {
    let left = uninitialized.ok_or("read of uninitialized local")?;
    let right = 1_i64
        .checked_div(divisor)
        .ok_or("invalid or overflowing integer arithmetic")?;
    left.checked_add(right)
        .ok_or("invalid or overflowing integer arithmetic")
}
