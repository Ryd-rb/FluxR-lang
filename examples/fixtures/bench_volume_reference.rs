fn lerp(a: i64, b: i64, weight: i64) -> Option<i64> {
    let delta = b.checked_sub(a)?;
    let offset = delta.checked_mul(weight)? / 1024;
    a.checked_add(offset)
}

pub(super) fn volume([a, b, c, d, e, f, g, h, x, y, z]: [i64; 11]) -> Option<i64> {
    let near = lerp(lerp(a, b, x)?, lerp(c, d, x)?, y)?;
    let far = lerp(lerp(e, f, x)?, lerp(g, h, x)?, y)?;
    lerp(near, far, z)
}
