pub fn run(values: &[i64], index: u64) -> Option<i64> {
    usize::try_from(index)
        .ok()
        .and_then(|index| values.get(index))
        .copied()
}
