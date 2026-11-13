pub fn run(value: Option<i64>, fallback: i64) -> i64 {
    value.unwrap_or(fallback)
}
