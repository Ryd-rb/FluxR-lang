// Matched scalar algorithm only; the generic loader/host is accounted separately.
pub fn sample(phase: f32, age: f32, velocity: f32) -> f32 {
    let wave = if phase < 0.5 {
        phase * 4.0 - 1.0
    } else {
        3.0 - phase * 4.0
    };
    (wave * velocity) * ((1.0 - age) * (1.0 - age))
}
