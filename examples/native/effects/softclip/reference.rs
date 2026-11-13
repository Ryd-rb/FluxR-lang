pub fn softclip(sample: f32, drive: f32) -> f32 {
    let scaled = sample * drive;
    scaled / (1.0 + scaled.abs())
}

pub fn mix(left: f32, right: f32, balance: f32) -> f32 {
    left * (1.0 - balance) + right * balance
}
