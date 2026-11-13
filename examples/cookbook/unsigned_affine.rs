pub fn run(x: u64, factor: u64, offset: u64) -> Option<u64> {
    x.checked_mul(factor)?.checked_add(offset)
}
