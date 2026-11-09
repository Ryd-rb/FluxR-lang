use crate::structured::Value::{self, *};
use macroquad::prelude::*;
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(super) struct Heightfield {
    origin: Vec2,
    step: f32,
    width: usize,
    height: usize,
    heights: Vec<f32>,
    pub meshes: Vec<Mesh>,
    pub detail: Option<Detail>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_uses_the_same_diagonal_as_the_rendered_grid() {
        let field = Heightfield {
            origin: vec2(-1.0, -1.0),
            step: 2.0,
            width: 2,
            height: 2,
            heights: vec![0.0, 1.0, 2.0, 8.0],
            meshes: Vec::new(),
            detail: None,
        };
        assert_eq!(field.elevation(0.0, 0.0), 4.0);
        assert_eq!(field.elevation(0.5, -0.5), 2.5);
        assert_eq!(field.elevation(-0.5, 0.5), 3.0);
        assert_eq!(field.elevation(1.0, 1.0), 8.0);
    }
}

pub(super) struct Detail {
    pub texture: Texture2D,
    pub bands: Vec4,
}

impl Heightfield {
    pub fn load(value: &Value, textures: &[Texture2D]) -> Result<Self> {
        let Record(fields) = value else {
            return Err("expected a heightfield record".into());
        };
        let [
            I64(x),
            I64(z),
            I64(step),
            I64(width),
            I64(height),
            List(samples),
            I64(texture),
            I64(period),
            I64(detail),
            I64(detail_period),
            I64(strength),
            I64(detail_strength),
        ] = fields.as_ref()
        else {
            return Err("expected heightfield origin, dimensions, samples and texture".into());
        };
        if !(2..=257).contains(width)
            || !(2..=257).contains(height)
            || samples.len() != (width * height) as usize
            || !(1..=100_000_000).contains(step)
            || !(1..=100_000_000).contains(period)
            || !(1..=100_000_000).contains(detail_period)
            || !(0..=1000).contains(strength)
            || !(0..=1000).contains(detail_strength)
        {
            return Err("invalid heightfield dimensions".into());
        }
        let texture_at = |index| -> Result<Option<&Texture2D>> {
            if index == -1 {
                Ok(None)
            } else {
                Ok(Some(
                    textures
                        .get(usize::try_from(index)?)
                        .ok_or("unknown heightfield texture")?,
                ))
            }
        };
        let texture = texture_at(*texture)?;
        let detail = texture_at(*detail)?.map(|texture| Detail {
            texture: texture.clone(),
            bands: vec4(
                *period as f32 / *detail_period as f32,
                *strength as f32 * 0.001,
                *detail_strength as f32 * 0.001,
                0.0,
            ),
        });
        let origin = vec2(*x as f32, *z as f32) * 0.001;
        let step = *step as f32 * 0.001;
        let width = *width as usize;
        let height = *height as usize;
        let period = *period as f32 * 0.001;
        let (heights, colors): (Vec<_>, Vec<_>) = samples
            .iter()
            .map(|value| {
                let Record(fields) = value else {
                    return Err("expected a heightfield sample");
                };
                let (y, rgb, opacity) = match fields.as_ref() {
                    [I64(y), I64(rgb @ 0..=0xff_ffff)] => (y, rgb, 1000),
                    [I64(y), I64(rgb @ 0..=0xff_ffff), I64(opacity @ 0..=1000)] => {
                        (y, rgb, *opacity)
                    }
                    _ => return Err("expected a height, RGB color and optional opacity"),
                };
                Ok((
                    *y as f32 * 0.001,
                    Color::from_rgba(
                        (rgb >> 16) as u8,
                        (rgb >> 8) as u8,
                        *rgb as u8,
                        (opacity * 255 / 1000) as u8,
                    ),
                ))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        let mut field = Self {
            origin,
            step,
            width,
            height,
            heights,
            meshes: Vec::new(),
            detail,
        };
        let vertex = |x: usize, z: usize| {
            let at = origin + step * vec2(x as f32, z as f32);
            let index = z * width + x;
            let lx = x.saturating_sub(1);
            let hx = (x + 1).min(width - 1);
            let lz = z.saturating_sub(1);
            let hz = (z + 1).min(height - 1);
            let dx = (field.heights[z * width + hx] - field.heights[z * width + lx])
                / (step * (hx - lx) as f32);
            let dz = (field.heights[hz * width + x] - field.heights[lz * width + x])
                / (step * (hz - lz) as f32);
            let mut v = Vertex::new2(
                vec3(at.x, field.heights[index], at.y),
                at / period,
                colors[index],
            );
            v.normal = vec3(-dx, 1.0, -dz).normalize().extend(1.0);
            v
        };
        field.meshes = (0..height - 1)
            .step_by(16)
            .map(|first| {
                let last = (first + 16).min(height - 1);
                let mut indices = Vec::with_capacity(6 * (last - first) * (width - 1));
                for z in 0..last - first {
                    for x in 0..width - 1 {
                        let a = (z * width + x) as u16;
                        let b = a + 1;
                        let c = a + width as u16;
                        let d = c + 1;
                        indices.extend([a, d, b, a, c, d]);
                    }
                }
                Mesh {
                    vertices: (first..=last)
                        .flat_map(|z| (0..width).map(move |x| vertex(x, z)))
                        .collect(),
                    indices,
                    texture: texture.cloned(),
                }
            })
            .collect();
        Ok(field)
    }

    pub fn elevation(&self, x: f32, z: f32) -> f32 {
        let p = (vec2(x, z) - self.origin) / self.step;
        let p = p.clamp(
            Vec2::ZERO,
            vec2((self.width - 1) as f32, (self.height - 1) as f32),
        );
        let x = (p.x as usize).min(self.width - 2);
        let z = (p.y as usize).min(self.height - 2);
        let u = p.x - x as f32;
        let v = p.y - z as f32;
        let index = z * self.width + x;
        let [a, b, c, d] =
            [index, index + 1, index + self.width, index + self.width + 1].map(|i| self.heights[i]);
        if u >= v {
            a + (b - a) * u + (d - b) * v
        } else {
            a + (d - c) * u + (c - a) * v
        }
    }

    pub fn normal(&self, x: f32, z: f32, radius: f32) -> Vec3 {
        let dx = self.elevation(x + radius, z) - self.elevation(x - radius, z);
        let dz = self.elevation(x, z + radius) - self.elevation(x, z - radius);
        vec3(-dx, 2.0 * radius, -dz).normalize()
    }
}
