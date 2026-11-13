pub fn run(values: &[u32]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut minimum = 0;
    for index in 1..values.len() {
        if f32::from_bits(values[index]) < f32::from_bits(values[minimum]) {
            minimum = index;
        }
    }
    minimum as u64
}
