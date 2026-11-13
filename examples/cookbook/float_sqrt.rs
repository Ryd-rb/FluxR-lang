pub fn run(bits: u32) -> u32 {
    let value = f32::from_bits(bits).sqrt();
    if value.is_nan() {
        0x7fc0_0000
    } else {
        value.to_bits()
    }
}
