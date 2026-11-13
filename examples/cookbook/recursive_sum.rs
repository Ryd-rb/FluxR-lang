pub fn run(n: i64) -> Option<i64> {
    if n <= 0 {
        return Some(0);
    }
    n.checked_add(run(n.checked_sub(1)?)?)
}
