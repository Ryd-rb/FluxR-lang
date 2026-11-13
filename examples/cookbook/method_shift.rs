pub struct Point {
    pub x: i64,
}
impl Point {
    pub fn shift(self, delta: i64) -> Option<i64> {
        self.x.checked_add(delta)
    }
}
pub fn run(point: Point, delta: i64) -> Option<i64> {
    point.shift(delta)
}
