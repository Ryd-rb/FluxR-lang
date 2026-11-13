//! Matched scalar reference with explicit binary32 operation order. This is a
//! timing/differential comparator, not an independent semantic authority.
#[derive(Clone, Copy)]
struct Dual {
    value: f32,
    tangent: f32,
}
impl Dual {
    fn add(self, other: Self) -> Self {
        Self {
            value: self.value + other.value,
            tangent: self.tangent + other.tangent,
        }
    }
    fn mul(self, other: Self) -> Self {
        Self {
            value: self.value * other.value,
            tangent: self.tangent * other.value + self.value * other.tangent,
        }
    }
    fn relu(self) -> Self {
        if self.value > 0. {
            self
        } else {
            Self {
                value: 0.,
                tangent: 0.,
            }
        }
    }
}
fn seeded(value: f32, direction: usize, parameter: usize) -> Dual {
    Dual {
        value,
        tangent: if direction == parameter { 1. } else { 0. },
    }
}
pub fn gradient(x: f32, y: f32, p: [f32; 4], direction: usize) -> f32 {
    let hidden = seeded(p[0], direction, 0)
        .mul(Dual {
            value: x,
            tangent: 0.,
        })
        .add(seeded(p[1], direction, 1))
        .relu();
    let output = hidden
        .mul(seeded(p[2], direction, 2))
        .add(seeded(p[3], direction, 3));
    let error = output.add(Dual {
        value: 0. - y,
        tangent: 0.,
    });
    error.value * error.tangent
}
pub fn train(dataset: &[(f32, f32)], epochs: usize, mut p: [f32; 4]) -> [f32; 4] {
    for _ in 0..epochs {
        for &(x, y) in dataset {
            let old = p;
            for direction in 0..4 {
                p[direction] = old[direction] - 0.03 * gradient(x, y, old, direction);
            }
        }
    }
    p
}
