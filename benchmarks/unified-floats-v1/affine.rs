fn canonical(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(0x7fc0_0000)
    } else {
        value
    }
}

pub fn run(x: u32) -> u32 {
    let product = canonical(f32::from_bits(x) * 0.5_f32);
    canonical(product + 1.25_f32).to_bits()
}
