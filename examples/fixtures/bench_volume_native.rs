// Generated from checked FluxR structured source. Edit the source program.

const NATIVE_PROGRAM_SHA256: [u8; 32] = [0x76, 0xa7, 0x22, 0x7f, 0x6f, 0x04, 0x56, 0x28, 0x0f, 0xa4, 0x32, 0x3b, 0xde, 0xdf, 0x98, 0xcc, 0x1a, 0xb5, 0xe0, 0xd9, 0xd0, 0xd5, 0xf5, 0xb9, 0x76, 0xb3, 0x6e, 0xf7, 0x70, 0x66, 0x10, 0xdc];

fn native_volume(v_a: i64, v_b: i64, v_c: i64, v_d: i64, v_e: i64, v_f: i64, v_g: i64, v_h: i64, v_x: i64, v_y: i64, v_z: i64) -> Option<i64> {
    let v_near: i64;
    let v_far: i64;
    v_near = native_lerp(native_lerp(v_a, v_b, v_x)?, native_lerp(v_c, v_d, v_x)?, v_y)?;
    v_far = native_lerp(native_lerp(v_e, v_f, v_x)?, native_lerp(v_g, v_h, v_x)?, v_y)?;
    Some(native_lerp(v_near, v_far, v_z)?)
}

fn native_lerp(v_a: i64, v_b: i64, v_t: i64) -> Option<i64> {
    Some((v_a).checked_add((((v_b).checked_sub(v_a)?).checked_mul(v_t)?).checked_div(1024i64)?)?)
}
