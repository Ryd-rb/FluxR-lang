use crate::structured::Value::{self, *};
use macroquad::prelude::*;
use std::{borrow::Cow, error::Error, fmt::Write};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

enum Draw<'a> {
    Clear(Color),
    Rect(Rect, Color),
    Border(Rect, Color),
    Text(Cow<'a, str>, Vec2, u16, Color, f32, Alignment),
    Camera(Camera3D),
    Box(Vec3, Vec3, Color),
    Line(Vec3, Vec3, Color),
    Overlay,
    Mesh(usize, Vec3, f32, f32, bool, Vec3),
    Stroke(Vec<Vec2>, f32, Color),
    Field(usize, Vec3, f32, f32),
    Placed(super::mesh::Placement),
    Swarm([i64; 2], Vec<super::swarm::SwarmSample>),
    Sphere(super::geometry::Sphere),
    Grid(usize, Vec3, f32),
}

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Center,
    Right,
}

fn color(n: i64) -> Result<Color> {
    Ok(Color::from_rgba(
        (n >> 24) as u8,
        (n >> 16) as u8,
        (n >> 8) as u8,
        u32::try_from(n)? as u8,
    ))
}

pub(super) fn point(value: &Value) -> Result<Vec3> {
    if let Record(fields) = value {
        let p = match fields.as_ref() {
            [F32(x), F32(y), F32(z)] => {
                vec3(f32::from_bits(*x), f32::from_bits(*y), f32::from_bits(*z))
            }
            [I64(x), I64(y), I64(z)] => vec3(*x as f32, *y as f32, *z as f32),
            _ => return Err("expected a 3D point".into()),
        };
        if p.is_finite() {
            return Ok(p);
        }
    }
    Err("expected a finite 3D point".into())
}

fn label<'a>(id: i64, args: &[Value], text: &'a [String]) -> Result<Cow<'a, str>> {
    let integer = |value: &Value| match value {
        I64(n) => Ok(*n),
        _ => Err("text arguments must be integers"),
    };
    if id == -1 {
        if args.len() > 256 {
            return Err("text request exceeds 256 characters".into());
        }
        let mut out = String::new();
        for value in args {
            let character =
                char::from_u32(u32::try_from(integer(value)?)?).ok_or("invalid character")?;
            if !character.is_control() {
                out.push(character);
            }
        }
        return Ok(Cow::Owned(out));
    }
    let template = text.get(usize::try_from(id)?).ok_or("unknown text index")?;
    if args.is_empty() {
        return Ok(Cow::Borrowed(template));
    }
    let mut parts = template.split("{}");
    let mut out = parts.next().unwrap_or_default().to_owned();
    for value in args {
        let rest = parts.next().ok_or("too many text arguments")?;
        write!(out, "{}{rest}", integer(value)?)?;
    }
    if parts.next().is_some() {
        return Err("too few text arguments".into());
    }
    Ok(Cow::Owned(out))
}

impl<'a> Draw<'a> {
    fn decode(value: &Value, text: &'a [String]) -> Result<Self> {
        match value {
            Variant(0, Some(v)) if matches!(v.as_ref(), I64(_)) => {
                let I64(rgba) = v.as_ref() else {
                    unreachable!()
                };
                Ok(Self::Clear(color(*rgba)?))
            }
            Variant(kind @ (1 | 2), Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let [I64(x), I64(y), I64(w), I64(h), I64(rgba)] = fields.as_ref() else {
                    return Err("expected a rectangle".into());
                };
                let rect = Rect::new(*x as f32, *y as f32, *w as f32, *h as f32);
                if *kind == 1 {
                    Ok(Self::Rect(rect, color(*rgba)?))
                } else {
                    Ok(Self::Border(rect, color(*rgba)?))
                }
            }
            Variant(3, Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let [
                    I64(id),
                    I64(x),
                    I64(y),
                    I64(size),
                    I64(rgba),
                    I64(width),
                    List(args),
                    alignment @ ..,
                ] = fields.as_ref()
                else {
                    return Err("expected a text request".into());
                };
                let label = label(*id, args, text)?;
                if !(1..=96).contains(size) || *width < 0 {
                    return Err("invalid text size or width".into());
                }
                let alignment = match alignment {
                    [] | [I64(0)] => Alignment::Left,
                    [I64(1)] => Alignment::Center,
                    [I64(2)] => Alignment::Right,
                    _ => return Err("invalid text alignment".into()),
                };
                Ok(Self::Text(
                    label,
                    vec2(*x as f32, *y as f32),
                    *size as u16,
                    color(*rgba)?,
                    *width as f32,
                    alignment,
                ))
            }
            Variant(4, Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let (eye, aim, up, fov, near, far, offset) = match fields.as_ref() {
                    [eye, aim, up, F32(fov), F32(near), F32(far)] => {
                        (eye, aim, up, fov, near, far, Vec3::ZERO)
                    }
                    [eye, aim, up, F32(fov), F32(near), F32(far), offset] => {
                        (eye, aim, up, fov, near, far, point(offset)? * 0.001)
                    }
                    _ => return Err("expected a perspective camera".into()),
                };
                let (position, target) = (point(eye)? + offset, point(aim)? + offset);
                let up = point(up)?
                    .try_normalize()
                    .ok_or("invalid camera up direction")?;
                let (fovy, z_near, z_far) = (
                    f32::from_bits(*fov),
                    f32::from_bits(*near),
                    f32::from_bits(*far),
                );
                if !(0.01..3.13).contains(&fovy)
                    || !z_far.is_finite()
                    || !(0.0 < z_near && z_near < z_far)
                    || (position - target).cross(up).length_squared() <= f32::EPSILON
                {
                    return Err("invalid perspective camera".into());
                }
                Ok(Self::Camera(Camera3D {
                    position,
                    target,
                    up,
                    fovy,
                    z_near,
                    z_far,
                    ..Default::default()
                }))
            }
            Variant(kind @ (5 | 6), Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let [a, b, I64(rgba)] = fields.as_ref() else {
                    return Err("expected a 3D primitive".into());
                };
                let (a, b, rgba) = (point(a)?, point(b)?, color(*rgba)?);
                if *kind == 5 && b.cmpgt(Vec3::ZERO).all() {
                    Ok(Self::Box(a, b, rgba))
                } else if *kind == 6 {
                    Ok(Self::Line(a, b, rgba))
                } else {
                    Err("box dimensions must be positive".into())
                }
            }
            Variant(7, None) => Ok(Self::Overlay),
            Variant(8, Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let [I64(asset), at, I64(yaw), I64(scale), Bool(ground), light] = fields.as_ref()
                else {
                    return Err("expected a mesh placement".into());
                };
                let light = point(light)?;
                if !(1..=1_000_000_000).contains(scale) || light.length_squared() < f32::EPSILON {
                    return Err("invalid mesh scale or light direction".into());
                }
                Ok(Self::Mesh(
                    usize::try_from(*asset)?,
                    point(at)? * 0.001,
                    *yaw as f32 * 0.001,
                    *scale as f32 * 0.000001,
                    *ground,
                    light,
                ))
            }
            Variant(9, Some(v)) if matches!(v.as_ref(), Record(_)) => {
                let Record(fields) = v.as_ref() else {
                    unreachable!()
                };
                let [List(points), I64(width), I64(rgba)] = fields.as_ref() else {
                    return Err("expected a 2D stroke".into());
                };
                if !(2..=128).contains(&points.len()) || !(1..=64).contains(width) {
                    return Err("invalid 2D stroke size".into());
                }
                let points = points
                    .iter()
                    .map(|value| match value {
                        Record(fields) => match fields.as_ref() {
                            [I64(x), I64(y)] => Ok(vec2(*x as f32, *y as f32)),
                            _ => Err("expected a 2D point"),
                        },
                        _ => Err("expected a 2D point"),
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(Self::Stroke(points, *width as f32, color(*rgba)?))
            }
            Variant(10, Some(v)) => {
                let Record(fields) = v.as_ref() else {
                    return Err("expected a heightfield draw".into());
                };
                let (field, light, opacity, edge) = match fields.as_ref() {
                    [I64(field), light] => (*field, light, 1.0, 0.0),
                    [I64(field), light, I64(opacity @ 0..=1000)] => {
                        (*field, light, *opacity as f32 * 0.001, 0.0)
                    }
                    [
                        I64(field),
                        light,
                        I64(opacity @ 0..=1000),
                        I64(edge @ 0..=1000),
                    ] => (*field, light, *opacity as f32 * 0.001, *edge as f32 * 0.001),
                    _ => {
                        return Err(
                            "expected a heightfield index, light and optional opacity".into()
                        );
                    }
                };
                Ok(Self::Field(
                    usize::try_from(field)?,
                    direction(light)?,
                    opacity,
                    edge,
                ))
            }
            Variant(11, Some(v)) => {
                use super::{animation::Sample, mesh::Placement};
                let Record(fields) = v.as_ref() else {
                    return Err("expected a posed mesh placement".into());
                };
                let [
                    I64(id),
                    I64(asset),
                    at,
                    I64(yaw),
                    I64(scale),
                    I64(floor),
                    Bool(follow),
                    light,
                    I64(clip),
                    I64(time),
                    I64(mode),
                    I64(vertical),
                    I64(coverage),
                    I64(shadow),
                ] = fields.as_ref()
                else {
                    return Err("expected mesh transform, contact, light and pose".into());
                };
                if !(1..=1_000_000_000).contains(scale)
                    || !(1..=4000).contains(vertical)
                    || !(0..=1000).contains(coverage)
                    || !(0..=1000).contains(shadow)
                    || !(0..=86_400_000).contains(time)
                {
                    return Err("invalid posed mesh parameters".into());
                }
                let clip = if *clip == -1 {
                    None
                } else {
                    let time = *time as f32 * 0.001;
                    let sample = match *mode {
                        0 if time <= 1.0 => Sample::Fraction(time),
                        1 | 2 => Sample::Seconds(time, *mode == 2),
                        _ => return Err("invalid clip sampling mode or time".into()),
                    };
                    Some((usize::try_from(*clip)?, sample))
                };
                Ok(Self::Placed(Placement {
                    id: *id,
                    asset: usize::try_from(*asset)?,
                    at: point(at)? * 0.001,
                    yaw: *yaw as f32 * 0.001,
                    scale: *scale as f32 * 0.000001,
                    floor: if *floor == -1 {
                        None
                    } else {
                        Some(usize::try_from(*floor)?)
                    },
                    follow: *follow,
                    light: direction(light)?,
                    clip,
                    vertical: *vertical as f32 * 0.001,
                    coverage: *coverage as f32 * 0.001,
                    shadow: *shadow as f32 * 0.001,
                }))
            }
            Variant(12, Some(v)) => {
                use super::swarm::SwarmSample;
                let Record(fields) = v.as_ref() else {
                    return Err("expected a swarm effect".into());
                };
                let [I64(source), I64(product), List(clouds)] = fields.as_ref() else {
                    return Err("expected swarm source, workpiece and clouds".into());
                };
                if clouds.len() > 8 {
                    return Err("too many swarm clouds".into());
                }
                let clouds = clouds
                    .iter()
                    .map(|value| {
                        let Record(fields) = value else {
                            return Err("expected a swarm cloud");
                        };
                        let [
                            I64(fill @ 0..=1000),
                            I64(recall @ 0..=1000),
                            I64(age @ 0..=86_400_000),
                            I64(density @ 0..=768),
                            I64(radius @ 0..=1000),
                        ] = fields.as_ref()
                        else {
                            return Err("invalid swarm cloud parameters");
                        };
                        Ok(SwarmSample {
                            fill: *fill as f32 * 0.001,
                            recall: *recall as f32 * 0.001,
                            age: *age as f32 * 0.001,
                            density: *density as f32,
                            radius: *radius as f32 * 0.001,
                        })
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(Self::Swarm([*source, *product], clouds))
            }
            Variant(13, Some(v)) => {
                let Record(fields) = v.as_ref() else {
                    return Err("expected a sphere record".into());
                };
                let [
                    center,
                    F32(radius),
                    north,
                    prime,
                    I64(texture),
                    I64(rgba),
                    light,
                    I64(emission @ 0..=1000),
                ] = fields.as_ref()
                else {
                    return Err("expected sphere dimensions, orientation and material".into());
                };
                let radius = f32::from_bits(*radius);
                let north = direction(north)?;
                let prime = point(prime)?;
                let prime = (prime - north * prime.dot(north))
                    .try_normalize()
                    .ok_or("sphere axes are parallel")?;
                if !radius.is_finite() || radius <= 0.0 {
                    return Err("invalid sphere radius".into());
                }
                Ok(Self::Sphere(super::geometry::Sphere {
                    center: point(center)?,
                    radius,
                    north,
                    prime,
                    texture: *texture,
                    color: color(*rgba)?,
                    light: direction(light)?,
                    emission: *emission as f32 * 0.001,
                }))
            }
            Variant(14, Some(v)) => {
                let Record(fields) = v.as_ref() else {
                    return Err("expected a surface grid draw".into());
                };
                let [I64(index), light, I64(opacity @ 0..=1000)] = fields.as_ref() else {
                    return Err("expected surface grid index, light and opacity".into());
                };
                Ok(Self::Grid(
                    usize::try_from(*index)?,
                    direction(light)?,
                    *opacity as f32 * 0.001,
                ))
            }
            _ => Err("invalid draw effect".into()),
        }
    }
}

fn direction(value: &Value) -> Result<Vec3> {
    point(value)?
        .try_normalize()
        .ok_or_else(|| "invalid light direction".into())
}

pub(super) fn render(
    draws: &[Value],
    text: &[String],
    font: Option<&Font>,
    dpi: f32,
    meshes: &mut super::mesh::Catalog,
    contacts: Option<&mut Vec<super::mesh::Contact>>,
) -> Result<(usize, Option<CameraReceipt>)> {
    let draws = draws
        .iter()
        .map(|value| Draw::decode(value, text))
        .collect::<Result<Vec<_>>>()?;
    let mut mesh_draws = 0;
    for draw in &draws {
        match draw {
            Draw::Mesh(asset, ..) => {
                meshes.prepare(*asset)?;
                mesh_draws += 1;
            }
            Draw::Placed(placement) => {
                meshes.prepare(placement.asset)?;
                mesh_draws += 1;
            }
            Draw::Field(..) | Draw::Grid(..) => {
                meshes.prepare_material()?;
                mesh_draws += 1;
            }
            Draw::Sphere(sphere) => {
                meshes.prepare_sphere(sphere.texture)?;
                mesh_draws += 1;
            }
            Draw::Swarm(..) => meshes.prepare_swarm()?,
            _ => {}
        }
    }
    // Glyph insertion can grow the atlas. Do it before queuing any draw using its texture or UVs.
    let sizes = draws
        .iter()
        .map(|draw| match draw {
            Draw::Text(text, _, size, _, width, _) => {
                let natural = measure_text(text, font, *size, 1.0).width;
                let scale = if *width > 0.0 && natural > *width {
                    *width / natural
                } else {
                    1.0
                };
                (scale, natural * scale)
            }
            _ => (1.0, 0.0),
        })
        .collect::<Vec<_>>();
    set_default_camera();
    let mut eye = Vec3::ZERO;
    let mut perspective = None;
    let mut receipt = None;
    let mut placed = Vec::new();
    for (draw, (scale, width)) in draws.into_iter().zip(sizes) {
        match draw {
            Draw::Clear(rgba) => clear_background(rgba),
            Draw::Rect(r, rgba) => draw_rectangle(r.x, r.y, r.w, r.h, rgba),
            Draw::Border(r, rgba) => draw_rectangle_lines(r.x, r.y, r.w, r.h, 1.0, rgba),
            Draw::Text(text, p, size, color, _, alignment) => {
                let snap = |v: f32| (v * dpi).round() / dpi;
                let offset = match alignment {
                    Alignment::Left => 0.0,
                    Alignment::Center => width * 0.5,
                    Alignment::Right => width,
                };
                draw_text_ex(
                    text,
                    snap(p.x - offset),
                    snap(p.y),
                    TextParams {
                        font,
                        font_size: size,
                        font_scale: scale,
                        color,
                        ..Default::default()
                    },
                );
            }
            Draw::Camera(camera) => {
                eye = camera.position;
                if contacts.is_some() {
                    receipt = Some(CameraReceipt {
                        eye: camera.position.to_array(),
                        aim: camera.target.to_array(),
                        up: camera.up.to_array(),
                        fov: camera.fovy,
                        near: camera.z_near,
                        far: camera.z_far,
                    });
                }
                set_camera(&camera);
                perspective = Some(camera);
            }
            Draw::Box(p, size, color) => draw_cube(p, size, None, color),
            Draw::Line(a, b, color) => draw_line_3d(a, b, color),
            Draw::Overlay => {
                perspective = None;
                set_default_camera();
            }
            Draw::Mesh(asset, at, yaw, scale, ground, light) => {
                meshes.draw(asset, at, yaw, scale, ground, [light, eye])
            }
            Draw::Field(field, light, opacity, edge) => {
                meshes.draw_field(field, light, eye, opacity, edge)?
            }
            Draw::Grid(index, light, opacity) => meshes.draw_grid(index, light, eye, opacity)?,
            Draw::Sphere(sphere) => meshes.draw_sphere(sphere, eye),
            Draw::Placed(placement) => {
                placed.push(meshes.draw_placed(placement, eye)?);
            }
            Draw::Swarm(ids, clouds) => meshes.draw_swarm(
                &placed,
                ids,
                perspective
                    .as_ref()
                    .ok_or("swarm requires a perspective camera")?,
                &clouds,
            )?,
            Draw::Stroke(points, width, color) => {
                for segment in points.windows(2) {
                    let [a, b] = [segment[0], segment[1]];
                    draw_line(a.x, a.y, b.x, b.y, width, color);
                }
            }
        }
    }
    if let Some(contacts) = contacts {
        contacts.extend(placed.into_iter().map(|placement| placement.contact));
    }
    Ok((mesh_draws, receipt))
}

#[derive(serde::Serialize)]
pub(super) struct CameraReceipt {
    eye: [f32; 3],
    aim: [f32; 3],
    up: [f32; 3],
    fov: f32,
    near: f32,
    far: f32,
}
