pub fn run(numerator: u64, denominator: u64) -> Option<u64> {
    numerator.checked_div(denominator)
}
