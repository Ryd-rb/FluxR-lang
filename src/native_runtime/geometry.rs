use super::{canvas::point, heightfield::Heightfield};
use crate::structured::Value::{self, *};
use macroquad::prelude::*;
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Default)]
pub(super) struct Geometry {
    pub fields: Vec<Heightfield>,
    pub grids: Vec<Grid>,
}

pub(super) struct Grid {
    pub meshes: Vec<Mesh>,
    pub detail: Option<Texture2D>,
    pub bands: Vec4,
    pub translucent: bool,
}

impl Geometry {
    pub fn load(value: &Value, textures: &[Texture2D]) -> Result<Self> {
        let Record(parts) = value else {
            return Err("expected application scene geometry".into());
        };
        let [List(fields), List(grids)] = parts.as_ref() else {
            return Err("expected heightfields and surface grids".into());
        };
        if fields.len() > 8 || grids.len() > 8 {
            return Err("too many application scene surfaces".into());
        }
        Ok(Self {
            fields: fields
                .iter()
                .map(|field| Heightfield::load(field, textures))
                .collect::<Result<_>>()?,
            grids: grids
                .iter()
                .map(|grid| Grid::load(grid, textures))
                .collect::<Result<_>>()?,
        })
    }
}

pub(super) fn texture(textures: &[Texture2D], index: i64) -> Result<Option<Texture2D>> {
    if index == -1 {
        return Ok(None);
    }
    Ok(Some(
        textures
            .get(usize::try_from(index)?)
            .ok_or("unknown geometry texture")?
            .clone(),
    ))
}

impl Grid {
    fn load(value: &Value, textures: &[Texture2D]) -> Result<Self> {
        let Record(fields) = value else {
            return Err("expected a surface grid record".into());
        };
        let [
            I64(w),
            I64(h),
            List(samples),
            I64(base),
            I64(period),
            I64(detail),
            I64(detail_period),
            I64(strength),
            I64(detail_strength),
        ] = fields.as_ref()
        else {
            return Err("expected a surface grid with dimensions, samples and material".into());
        };
        if !(2..=1025).contains(w)
            || !(2..=1025).contains(h)
            || samples.len() != (w * h) as usize
            || samples.len() > 24_000
            || !(1..=100_000_000).contains(period)
            || !(1..=100_000_000).contains(detail_period)
            || !(0..=1000).contains(strength)
            || !(0..=1000).contains(detail_strength)
        {
            return Err("invalid surface grid dimensions or material".into());
        }
        let (w, h) = (*w as usize, *h as usize);
        let mut vertices = Vec::with_capacity(samples.len());
        let mut translucent = false;
        for sample in samples.iter() {
            let Record(fields) = sample else {
                return Err("expected a surface grid sample".into());
            };
            let [
                position,
                I64(rgb @ 0..=0xff_ffff),
                I64(opacity @ 0..=1000),
                I64(detail @ 0..=1000),
            ] = fields.as_ref()
            else {
                return Err("expected a surface position, RGB color and opacity".into());
            };
            translucent |= *opacity < 1000;
            let p = point(position)?;
            let mut vertex = Vertex::new2(
                p,
                vec2(p.x, p.z) * (1000.0 / *period as f32),
                Color::from_rgba(
                    (rgb >> 16) as u8,
                    (rgb >> 8) as u8,
                    *rgb as u8,
                    (*opacity * 255 / 1000) as u8,
                ),
            );
            vertex.normal = vec4(0.0, 0.0, 0.0, *detail as f32 * 0.001);
            vertices.push(vertex);
        }
        let rows = 32_760 / (6 * (w - 1));
        let mut batches = Vec::new();
        for first in (0..h - 1).step_by(rows) {
            let end = (first + rows).min(h - 1);
            let mut indices = Vec::with_capacity(6 * (end - first) * (w - 1));
            for z in first..end {
                for x in 0..w - 1 {
                    let a = z * w + x;
                    let [a, b, c, d] = [a, a + 1, a + w, a + w + 1];
                    for [a, b, c] in [[a, d, b], [a, c, d]] {
                        let normal = (vertices[b].position - vertices[a].position)
                            .cross(vertices[c].position - vertices[a].position);
                        for index in [a, b, c] {
                            vertices[index].normal += normal.extend(0.0);
                            indices.push((index - first * w) as u16);
                        }
                    }
                }
            }
            batches.push((first, end, indices));
        }
        for v in &mut vertices {
            v.normal = v
                .normal
                .truncate()
                .try_normalize()
                .unwrap_or(Vec3::Y)
                .extend(v.normal.w);
        }
        let base = texture(textures, *base)?;
        Ok(Self {
            meshes: batches
                .into_iter()
                .map(|(first, end, indices)| Mesh {
                    vertices: vertices[first * w..(end + 1) * w].to_vec(),
                    indices,
                    texture: base.clone(),
                })
                .collect(),
            detail: texture(textures, *detail)?,
            translucent,
            bands: vec4(
                *period as f32 / *detail_period as f32,
                *strength as f32 * 0.001,
                *detail_strength as f32 * 0.001,
                0.0,
            ),
        })
    }
}

pub(super) struct Sphere {
    pub center: Vec3,
    pub radius: f32,
    pub north: Vec3,
    pub prime: Vec3,
    pub texture: i64,
    pub color: Color,
    pub light: Vec3,
    pub emission: f32,
}

pub(super) fn sphere_mesh(texture: Option<Texture2D>) -> Mesh {
    use std::f32::consts::{PI, TAU};
    const WIDTH: u16 = 97;
    const HEIGHT: u16 = 49;
    let mut vertices = Vec::with_capacity((WIDTH * HEIGHT).into());
    let mut indices = Vec::with_capacity(6 * ((WIDTH - 1) * (HEIGHT - 1)) as usize);
    for y in 0..HEIGHT {
        let v = f32::from(y) / f32::from(HEIGHT - 1);
        let (radial, north) = (v * PI).sin_cos();
        for x in 0..WIDTH {
            let u = f32::from(x) / f32::from(WIDTH - 1);
            let (sin, cos) = ((u - 0.5) * TAU).sin_cos();
            let p = vec3(cos * radial, north, sin * radial);
            let mut vertex = Vertex::new2(p, vec2(u, v), WHITE);
            vertex.normal = p.extend(0.0);
            vertices.push(vertex);
            if x + 1 < WIDTH && y + 1 < HEIGHT {
                let a = y * WIDTH + x;
                indices.extend([a, a + 1, a + WIDTH + 1, a, a + WIDTH + 1, a + WIDTH]);
            }
        }
    }
    Mesh {
        vertices,
        indices,
        texture,
    }
}
