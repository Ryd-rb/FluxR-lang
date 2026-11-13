fn canonical(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(0x7fc0_0000)
    } else {
        value
    }
}

pub fn run(x: u32, y: u32) -> u32 {
    let difference = canonical(f32::from_bits(x) - f32::from_bits(y));
    canonical(difference / 3.0_f32).to_bits()
}
