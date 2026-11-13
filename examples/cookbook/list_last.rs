pub fn run(values: &[i64], fallback: i64) -> i64 {
    values.last().copied().unwrap_or(fallback)
}
