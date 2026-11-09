pub fn pick(values: impl IntoIterator<Item = f32>) -> u64 {
    let mut values = values.into_iter();
    let Some(mut best) = values.next() else {
        return 0;
    };
    let mut winner = 0;
    for (offset, value) in values.enumerate() {
        if value < best {
            best = value;
            winner = offset as u64 + 1;
        }
    }
    winner
}

pub fn decide([a, b, c, x]: [i64; 4]) -> Option<i64> {
    if a < 0 {
        if b < 0 {
            if c < 0 {
                x.checked_add(1)
            } else {
                x.checked_sub(1)
            }
        } else if c < 0 {
            x.checked_mul(2)
        } else {
            Some(x / 2)
        }
    } else if b < 0 {
        if c < 0 {
            x.checked_add(3)
        } else {
            x.checked_sub(3)
        }
    } else if c < 0 {
        x.checked_mul(4)
    } else {
        Some(x / 4)
    }
}
