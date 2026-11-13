pub fn run(numerator: i64, denominator: i64) -> Option<bool> {
    if denominator == 0 {
        return Some(false);
    }
    Some(numerator.checked_div(denominator)? > 0)
}
