pub struct Point {
    pub x: i64,
    pub y: i64,
}
pub fn run(x: i64) -> Option<Point> {
    Some(Point {
        x,
        y: x.checked_add(1)?,
    })
}
