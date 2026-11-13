pub fn run(enabled: bool, base: i64, delta: i64) -> Option<i64> {
    if enabled {
        base.checked_add(delta)
    } else {
        base.checked_sub(delta)
    }
}
