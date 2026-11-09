pub fn leaf(x: i64, y: i64) -> Option<i64> {
    x.checked_mul(y)?.checked_add(17)?.checked_rem(997)
}

pub use leaf as rule;

pub fn counter(mut total: i64, count: i64, bonus: i64) -> Option<i64> {
    for i in 0..count {
        total = total.checked_add(if i % 2 == 0 { bonus } else { i.checked_mul(2)? })?;
    }
    Some(total)
}

pub fn gate(mut fact: impl FnMut(usize) -> bool) -> bool {
    fact(0) && fact(1) && fact(2) && fact(3)
}

pub fn timeline(t: u32, tracks: u16) -> impl ExactSizeIterator<Item = i32> {
    let t = (t % 1000) as i32;
    (0..tracks).map(move |id| i32::from(id) + t.min(1000 - t))
}
