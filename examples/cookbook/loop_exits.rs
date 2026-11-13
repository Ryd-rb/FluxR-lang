pub fn run(mut n: i64) -> Option<i64> {
    let mut total = 0_i64;
    while n > 0 {
        n = n.checked_sub(1)?;
        if n == 2 {
            continue;
        }
        if n == 0 {
            break;
        }
        total = total.checked_add(n)?;
    }
    Some(total)
}
