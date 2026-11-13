pub struct Point {
    pub x: i64,
    pub y: i64,
}
pub fn run(mut point: Point, delta: i64) -> Option<Point> {
    point.x = point.x.checked_add(delta)?;
    Some(point)
}
