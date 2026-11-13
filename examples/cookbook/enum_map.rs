pub fn run(value: Option<i64>, delta: i64) -> Option<Option<i64>> {
    match value {
        None => Some(None),
        Some(value) => Some(Some(value.checked_add(delta)?)),
    }
}
