pub fn run(x: i64, low: i64, high: i64) -> i64 {
    if x < low {
        return low;
    }
    if high < x {
        return high;
    }
    x
}
