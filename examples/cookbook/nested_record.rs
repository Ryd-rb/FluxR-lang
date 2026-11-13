pub struct Point {
    pub x: i64,
    pub y: i64,
}
pub struct Envelope {
    pub point: Point,
}
pub fn run(value: Envelope) -> Option<i64> {
    value.point.x.checked_add(value.point.y)
}
