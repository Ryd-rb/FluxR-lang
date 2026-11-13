pub fn run(values: &[i64], needle: i64) -> bool {
    values.iter().rev().any(|&value| value == needle)
}
