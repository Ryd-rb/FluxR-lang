pub fn run(base: i64, mut exponent: u64) -> Option<i64> {
    let mut product = 1_i64;
    while exponent > 0 {
        product = product.checked_mul(base)?;
        exponent = exponent.checked_sub(1)?;
    }
    Some(product)
}
