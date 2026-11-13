pub fn run(rows: &[Vec<i64>]) -> Option<i64> {
    rows.last()?.last().copied()
}
