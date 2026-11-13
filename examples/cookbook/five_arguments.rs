pub fn run(a: i64, b: i64, c: i64, d: i64, e: i64) -> Option<i64> {
    a.checked_add(b)?
        .checked_add(c)?
        .checked_add(d)?
        .checked_add(e)
}
