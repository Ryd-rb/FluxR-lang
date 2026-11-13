fn canonical(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(0x7fc0_0000)
    } else {
        value
    }
}

pub fn run(x: u32, n: i64) -> Option<u32> {
    let mut x = f32::from_bits(x);
    let mut i = 0_i64;
    while i < n {
        x = canonical(x * 1.5_f32);
        x = canonical(x + 0.25_f32);
        i = i.checked_add(1)?;
    }
    Some(x.to_bits())
}
