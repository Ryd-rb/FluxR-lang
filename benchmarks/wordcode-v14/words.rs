// Matched optional shared Rust vocabulary. Charge this text when retrieved.
pub fn step(x: i64, y: i64, z: i64) -> Option<i64> { x.checked_mul(y)?.checked_add(z) }
pub fn square(x: i64) -> Option<i64> { x.checked_mul(x) }
pub fn twice(x: i64) -> Option<i64> { x.checked_mul(2) }
pub fn sum(x: i64, y: i64, z: i64) -> Option<i64> { x.checked_add(y)?.checked_add(z) }
pub fn repeat(n: i64, mut x: i64) -> Option<i64> { for _ in 0..n { x = x.checked_add(1)?; } Some(x) }
pub fn walk(mut x: i64, n: i64, s: i64) -> Option<i64> { while x < n { x = x.checked_add(s)?; } Some(x) }
pub fn forest(x: i64) -> Option<i64> { x.checked_pow(256) }
