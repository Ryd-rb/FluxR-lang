pub fn smooth(sample: f32, alpha: f32, previous: f32) -> f32 {
    previous + alpha * (sample - previous)
}
pub fn residual(sample: f32, low: f32) -> f32 {
    sample - low
}
pub fn envelope(sample: f32, alpha: f32, previous: f32) -> f32 {
    previous + alpha * (sample.abs() - previous)
}
pub fn gain(sample: f32, level: f32) -> f32 {
    sample * level
}
