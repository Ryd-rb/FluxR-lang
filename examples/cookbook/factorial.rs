pub fn run(n: u64) -> Option<u64> {
    let mut product = 1_u64;
    let mut factor = 2_u64;
    while factor <= n {
        product = product.checked_mul(factor)?;
        factor = factor.checked_add(1)?;
    }
    Some(product)
}
