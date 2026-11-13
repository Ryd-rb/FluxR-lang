pub fn run(values: &[i64], value: i64) -> Vec<i64> {
    let mut result = values.to_vec();
    result.push(value);
    result.push(7);
    result.pop();
    result
}
