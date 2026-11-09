include!("bench_volume_native.rs");

pub(super) const IDENTITY: [u8; 32] = NATIVE_PROGRAM_SHA256;

pub(super) fn run([a, b, c, d, e, f, g, h, x, y, z]: [i64; 11]) -> Option<i64> {
    native_volume(a, b, c, d, e, f, g, h, x, y, z)
}
