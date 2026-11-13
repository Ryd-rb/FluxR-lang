pub fn run(a: i64, b: i64, c: i64, x: i64) -> Option<i64> {
    if a < 0 {
        if b < 0 {
            if c < 0 {
                x.checked_add(1)
            } else {
                x.checked_sub(1)
            }
        } else if c < 0 {
            x.checked_mul(2)
        } else {
            x.checked_div(2)
        }
    } else if b < 0 {
        if c < 0 {
            x.checked_add(3)
        } else {
            x.checked_sub(3)
        }
    } else if c < 0 {
        x.checked_mul(4)
    } else {
        x.checked_div(4)
    }
}
