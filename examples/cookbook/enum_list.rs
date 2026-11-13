pub fn run(value: Option<&[i64]>) -> bool {
    value.map_or(true, |values| values.is_empty())
}
