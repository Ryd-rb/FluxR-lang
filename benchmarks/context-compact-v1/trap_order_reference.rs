fn calc(x: i64, y: i64) -> Option<i64> {
    x.checked_add(1)?.checked_div(y)
}
