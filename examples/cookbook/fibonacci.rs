pub fn run(mut n: i64) -> Option<i64> {
    let mut a = 0_i64;
    let mut b = 1_i64;
    while n > 0 {
        let next = a.checked_add(b)?;
        a = b;
        b = next;
        n = n.checked_sub(1)?;
    }
    Some(a)
}
