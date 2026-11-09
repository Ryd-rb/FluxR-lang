//! Cached rigid GLB and heightfield drawing.
use super::{
    animation::{Clip, Node, Sample},
    geometry::{self, Geometry, Sphere},
    heightfield::Heightfield,
    swarm::{Swarm, SwarmSample, Workpiece},
};
use gltf::{Gltf, buffer, image, mesh::Mode};
use macroquad::prelude::*;
use std::{collections::HashMap, error::Error, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BATCH: usize = 30_000;

pub(super) struct Source {
    pub path: PathBuf,
    pub clips: Vec<String>,
    pub anchors: Vec<String>,
}

pub(super) struct Placement {
    pub id: i64,
    pub asset: usize,
    pub at: Vec3,
    pub yaw: f32,
    pub scale: f32,
    pub floor: Option<usize>,
    pub follow: bool,
    pub light: Vec3,
    pub clip: Option<(usize, Sample)>,
    pub vertical: f32,
    pub coverage: f32,
    pub shadow: f32,
}

#[derive(serde::Serialize)]
pub(super) struct Contact {
    id: i64,
    asset: usize,
    at: [f32; 3],
    ground: Option<f32>,
    up: [f32; 3],
    clip: Option<usize>,
    coverage: f32,
}

pub(super) struct Placed {
    pub contact: Contact,
    root: Mat4,
    low: Vec3,
    high: Vec3,
    anchors: [Option<Vec3>; 2],
}

pub(super) struct Catalog {
    sources: Vec<Source>,
    models: Vec<Option<Model>>,
    material: Option<(Material, Material)>,
    pub geometry: Geometry,
    pub textures: Vec<Texture2D>,
    spheres: HashMap<i64, Mesh>,
    swarm: Option<Swarm>,
}

struct Model {
    parts: Vec<Part>,
    nodes: Vec<Node>,
    clips: Vec<Clip>,
    anchors: Vec<(usize, Vec3)>,
    minimum_y: f32,
    low: Vec3,
    high: Vec3,
}

struct Part {
    node: usize,
    mesh: Mesh,
    surface: Vec4,
    emission: Vec3,
    blend: bool,
    center: Vec3,
}

impl Catalog {
    pub fn new(sources: Vec<Source>, geometry: Geometry, textures: Vec<Texture2D>) -> Self {
        Self {
            models: (0..sources.len()).map(|_| None).collect(),
            sources,
            material: None,
            geometry,
            textures,
            spheres: HashMap::new(),
            swarm: None,
        }
    }

    pub fn prepare(&mut self, asset: usize) -> Result<()> {
        let model = self.models.get_mut(asset).ok_or("unknown mesh index")?;
        if model.is_none() {
            let Source {
                path,
                clips,
                anchors,
            } = &self.sources[asset];
            let bytes = super::application::read(path, 32 * 1024 * 1024)?;
            *model = Some(
                Model::load(&bytes, clips, anchors)
                    .map_err(|err| format!("{}: {err}", path.display()))?,
            );
        }
        self.prepare_material()
    }

    pub fn prepare_material(&mut self) -> Result<()> {
        if self.material.is_none() {
            use macroquad::miniquad::{
                BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams,
            };
            let make = |blend: bool| {
                load_material(
                    ShaderSource::Glsl {
                        vertex: r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;
uniform mat4 Model;
uniform mat4 Projection;
varying lowp vec4 tint;
varying mediump vec2 uv;
varying highp vec3 world;
varying mediump vec3 worldNormal;
varying mediump float groundDetail;
void main() {
    worldNormal = normalize(mat3(Model) * normal.xyz);
    groundDetail = normal.w;
    tint = color0 / 255.0;
    uv = texcoord;
    vec4 p = Model * vec4(position, 1.0);
    world = p.xyz;
    gl_Position = Projection * p;
}"#,
                        fragment: r#"#version 100
precision mediump float;
uniform sampler2D Texture;
uniform sampler2D Detail;
uniform vec3 Light;
uniform vec3 Eye;
uniform vec4 Surface;
uniform vec3 Emission;
uniform float Coverage;
uniform float Opacity;
uniform float VertexAlpha;
uniform vec4 Reveal;
uniform vec4 Bands;
varying lowp vec4 tint;
varying mediump vec2 uv;
varying highp vec3 world;
varying mediump vec3 worldNormal;
varying mediump float groundDetail;
void main() {
    vec4 texel = texture2D(Texture, Surface.w > 0.0 ? fract(uv) : uv);
    float alpha = tint.a * texel.a;
    if (alpha < Surface.z) discard;
    float glow = 0.0;
    if (Coverage < 1.0) {
        float grain = fract(dot(floor(world * 64.0), vec3(0.7548777, 0.5698403, 0.32831)));
        float edge = Coverage - dot(world, Reveal.xyz) - Reveal.w + (grain - 0.5) * 0.08;
        if (edge < 0.0) discard;
        glow = 1.0 - smoothstep(0.0, 0.055, edge);
    }
    vec3 base = tint.rgb * pow(texel.rgb, vec3(2.2));
    vec3 n = normalize(worldNormal);
    vec3 sun = normalize(Light);
    if (Surface.w < 0.0) {
        vec3 shade = pow(texel.rgb * Bands.rgb, vec3(2.2));
        shade *= mix(0.105 + 0.895 * max(dot(n, sun), 0.0), 1.0, Emission.x);
        gl_FragColor = vec4(pow(shade, vec3(1.0 / 2.2)), alpha);
        return;
    }
    if (Surface.w > 0.0) {
        vec3 detail = texture2D(Detail, fract(uv * Bands.x)).rgb;
        vec3 grain = texture2D(Texture, fract(uv + 0.27 * detail.rg)).rgb;
        vec3 tone = tint.rgb * (1.0 + groundDetail * (Bands.y * (grain.r - 0.49) + Bands.z * (detail.r - 0.51)));
        base = pow(max(tone, vec3(0.0)), vec3(2.2));
        alpha = mix(1.0, tint.a, VertexAlpha);
    }
    float day = smoothstep(-0.05, 0.12, sun.y);
    float direct = day * max(dot(n, sun), 0.0);
    vec3 halfAngle = normalize(normalize(Eye - world) + sun);
    float highlight = pow(max(dot(n, halfAngle), 0.0), mix(96.0, 6.0, Surface.x));
    vec3 specular = mix(vec3(0.045), base, Surface.y) * highlight * direct;
    vec3 linear = base * (mix(0.13, 0.40, day) + direct * 0.60) + specular + Emission;
    linear = mix(linear, vec3(0.38, 0.67, 0.75), 0.48 * glow);
    gl_FragColor = vec4(pow(max(linear, vec3(0.0)), vec3(1.0 / 2.2)), alpha * Opacity);
}"#,
                    },
                    MaterialParams {
                        uniforms: vec![
                            UniformDesc::new("Light", UniformType::Float3),
                            UniformDesc::new("Eye", UniformType::Float3),
                            UniformDesc::new("Surface", UniformType::Float4),
                            UniformDesc::new("Emission", UniformType::Float3),
                            UniformDesc::new("Coverage", UniformType::Float1),
                            UniformDesc::new("Opacity", UniformType::Float1),
                            UniformDesc::new("VertexAlpha", UniformType::Float1),
                            UniformDesc::new("Reveal", UniformType::Float4),
                            UniformDesc::new("Bands", UniformType::Float4),
                        ],
                        textures: vec!["Detail".to_owned()],
                        pipeline_params: PipelineParams {
                            depth_write: !blend,
                            depth_test: Comparison::LessOrEqual,
                            color_blend: blend.then(|| {
                                BlendState::new(
                                    Equation::Add,
                                    BlendFactor::Value(BlendValue::SourceAlpha),
                                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                                )
                            }),
                            ..Default::default()
                        },
                    },
                )
            };
            self.material = Some((make(false)?, make(true)?));
        }
        Ok(())
    }

    pub fn draw(
        &self,
        asset: usize,
        mut at: Vec3,
        yaw: f32,
        scale: f32,
        ground: bool,
        [light, eye]: [Vec3; 2],
    ) {
        let model = self.models[asset].as_ref().expect("prepared mesh");
        if ground {
            at.y -= model.minimum_y * scale;
        }
        let matrix = Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            Quat::from_rotation_y(yaw),
            at,
        );
        self.draw_model(model, matrix, None, light, eye, 1.0);
    }

    pub fn draw_field(
        &self,
        field: usize,
        light: Vec3,
        eye: Vec3,
        opacity: f32,
        edge: f32,
    ) -> Result<()> {
        let field = self
            .geometry
            .fields
            .get(field)
            .ok_or("unknown heightfield")?;
        let materials = self.material.as_ref().expect("prepared material");
        let material = if opacity < 1.0 || edge > 0.0 {
            &materials.1
        } else {
            &materials.0
        };
        let bands = field.detail.as_ref().map_or(Vec4::ZERO, |detail| {
            material.set_texture("Detail", detail.texture.clone());
            detail.bands
        });
        material.set_uniform("Bands", bands);
        set_material(
            material,
            light,
            eye,
            vec4(1.0, 0.0, 0.0, 1.0),
            Vec3::ZERO,
            1.0,
        );
        material.set_uniform("Opacity", opacity);
        material.set_uniform("VertexAlpha", edge);
        for mesh in &field.meshes {
            draw_mesh(mesh);
        }
        gl_use_default_material();
        Ok(())
    }

    pub fn draw_grid(&self, index: usize, light: Vec3, eye: Vec3, opacity: f32) -> Result<()> {
        let grid = self
            .geometry
            .grids
            .get(index)
            .ok_or("unknown surface grid")?;
        let materials = self.material.as_ref().expect("prepared material");
        let material = if opacity < 1.0 || grid.translucent {
            &materials.1
        } else {
            &materials.0
        };
        if let Some(detail) = &grid.detail {
            material.set_texture("Detail", detail.clone());
        }
        material.set_uniform("Bands", grid.bands);
        set_material(
            material,
            light,
            eye,
            vec4(1.0, 0.0, 0.0, 1.0),
            Vec3::ZERO,
            1.0,
        );
        material.set_uniform("Opacity", opacity);
        for mesh in &grid.meshes {
            draw_mesh(mesh);
        }
        gl_use_default_material();
        Ok(())
    }

    pub fn prepare_sphere(&mut self, index: i64) -> Result<()> {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.spheres.entry(index) {
            entry.insert(geometry::sphere_mesh(geometry::texture(
                &self.textures,
                index,
            )?));
        }
        self.prepare_material()
    }

    pub fn draw_sphere(&self, sphere: Sphere, eye: Vec3) {
        let material = &self.material.as_ref().expect("prepared material").0;
        let Sphere {
            center,
            radius,
            north,
            prime,
            texture,
            color,
            light,
            emission,
        } = sphere;
        material.set_uniform("Bands", vec4(color.r, color.g, color.b, color.a));
        set_material(
            material,
            light,
            eye,
            vec4(1.0, 0.0, 0.0, -1.0),
            Vec3::splat(emission),
            1.0,
        );
        let east = prime.cross(north);
        let model = Mat4::from_cols(
            (prime * radius).extend(0.0),
            (north * radius).extend(0.0),
            (east * radius).extend(0.0),
            center.extend(1.0),
        );
        unsafe { get_internal_gl().quad_gl.push_model_matrix(model) };
        draw_mesh(&self.spheres[&texture]);
        unsafe { get_internal_gl().quad_gl.pop_model_matrix() };
        gl_use_default_material();
    }

    pub fn draw_placed(&self, placement: Placement, eye: Vec3) -> Result<Placed> {
        let Placement {
            id,
            asset,
            mut at,
            yaw,
            scale,
            floor,
            follow,
            light,
            clip,
            vertical,
            coverage,
            shadow,
        } = placement;
        let model = self.models[asset].as_ref().expect("prepared mesh");
        let mut rotation = Quat::from_rotation_y(yaw);
        let mut ground = None;
        if let Some(floor) = floor {
            let floor = self
                .geometry
                .fields
                .get(floor)
                .ok_or("unknown placement heightfield")?;
            let y = floor.elevation(at.x, at.z);
            ground = Some(y);
            at.y += y;
            if follow {
                rotation =
                    Quat::from_rotation_arc(Vec3::Y, floor.normal(at.x, at.z, scale)) * rotation;
            }
            if shadow > 0.0 {
                self.draw_shadow(
                    floor,
                    at,
                    (model.high - model.low) * scale,
                    light,
                    eye,
                    shadow * coverage,
                );
            }
        }
        let pose = clip
            .map(|(index, sample)| {
                model
                    .clips
                    .get(index)
                    .map(|clip| clip.matrices(&model.nodes, sample))
                    .ok_or("unknown model clip")
            })
            .transpose()?;
        let matrix = Mat4::from_scale_rotation_translation(
            vec3(scale, scale * vertical, scale),
            rotation,
            at,
        );
        self.draw_model(model, matrix, pose.as_deref(), light, eye, coverage);
        let mut anchors = [None; 2];
        for (out, &(node, offset)) in anchors.iter_mut().zip(&model.anchors) {
            let transform = pose
                .as_ref()
                .map_or(model.nodes[node].bind, |nodes| nodes[node]);
            *out = Some((matrix * transform).transform_point3(offset));
        }
        Ok(Placed {
            contact: Contact {
                id,
                asset,
                at: at.to_array(),
                ground,
                up: (rotation * Vec3::Y).to_array(),
                clip: placement.clip.map(|(index, _)| index),
                coverage,
            },
            root: matrix,
            low: model.low,
            high: model.high,
            anchors,
        })
    }

    pub fn prepare_swarm(&mut self) -> Result<()> {
        if self.swarm.is_none() {
            self.swarm = Some(Swarm::new()?);
        }
        Ok(())
    }

    pub fn draw_swarm(
        &self,
        placed: &[Placed],
        ids: [i64; 2],
        camera: &Camera3D,
        clouds: &[SwarmSample],
    ) -> Result<()> {
        let [source, product] = ids.map(|id| placed.iter().find(|p| p.contact.id == id));
        let (Some(source), Some(product)) = (source, product) else {
            return Err("swarm references an absent posed mesh".into());
        };
        let [first, second] = source.anchors;
        let first = first.unwrap_or_else(|| {
            let mut center = (source.low + source.high) * 0.5;
            center.y = source.high.y;
            source.root.transform_point3(center)
        });
        let [width, height, depth] = (product.high - product.low).to_array();
        let bed = Workpiece {
            bed: product.root.transform_point3(product.low),
            width: product.root.transform_vector3(Vec3::X * width),
            depth: product.root.transform_vector3(Vec3::Z * depth),
            height: product.root.transform_vector3(Vec3::Y * height),
        };
        self.swarm.as_ref().expect("prepared swarm").draw(
            [first, second.unwrap_or(first)],
            &bed,
            camera.position,
            camera.matrix(),
            clouds,
        );
        Ok(())
    }

    fn draw_shadow(
        &self,
        floor: &Heightfield,
        at: Vec3,
        size: Vec3,
        light: Vec3,
        eye: Vec3,
        opacity: f32,
    ) {
        let material = &self.material.as_ref().expect("prepared material").1;
        set_material(
            material,
            light,
            eye,
            vec4(1.0, 1.0, 0.0, 0.0),
            Vec3::ZERO,
            1.0,
        );
        let foot = vec2(at.x, at.z);
        let away = -vec2(light.x, light.z);
        let axis = away.try_normalize().unwrap_or(Vec2::X);
        let radius = size.x.max(size.z) * 0.46;
        let stamp = |center: Vec2, major: f32, minor: f32, alpha: f32| {
            let point = |p: Vec2, a| {
                let mut vertex = Vertex::new2(
                    vec3(p.x, floor.elevation(p.x, p.y) + 0.016, p.y),
                    Vec2::ZERO,
                    Color::new(0.0, 0.0, 0.0, a),
                );
                vertex.normal = Vec3::Y.extend(0.0);
                vertex
            };
            let mut vertices = vec![point(center, alpha)];
            let mut indices = Vec::with_capacity(16 * 3);
            for i in 0..16_u16 {
                let t = Vec2::from_angle(i as f32 * std::f32::consts::TAU / 16.0);
                vertices.push(point(
                    center + axis * (t.x * major) + axis.perp() * (t.y * minor),
                    0.0,
                ));
                indices.extend([0, 1 + i, 1 + (i + 1) % 16]);
            }
            draw_mesh(&Mesh {
                vertices,
                indices,
                texture: None,
            });
        };
        if light.y > 0.0 {
            let reach = (size.y * away.length() / light.y.max(0.12)).min(3.0 * size.y);
            stamp(
                foot + axis * (0.42 * reach),
                radius + reach * 0.49,
                radius,
                opacity * 0.42,
            );
        }
        stamp(foot, radius, radius * 0.83, opacity * 0.55);
        gl_use_default_material();
    }

    fn draw_model(
        &self,
        model: &Model,
        root: Mat4,
        pose: Option<&[Mat4]>,
        light: Vec3,
        eye: Vec3,
        coverage: f32,
    ) {
        let (opaque, blend) = self.material.as_ref().expect("prepared material");
        if coverage < 1.0 {
            let depth = root.transform_vector3(Vec3::Z * (model.high.z - model.low.z).max(0.001));
            let front = root.transform_point3(Vec3::Z * model.high.z);
            let normal = -depth / depth.length_squared();
            let reveal = normal.extend(-front.dot(normal));
            opaque.set_uniform("Reveal", reveal);
            blend.set_uniform("Reveal", reveal);
        }
        let matrix =
            |part: &Part| root * pose.map_or(model.nodes[part.node].bind, |nodes| nodes[part.node]);
        let draw = |part: &Part, material: &Material| {
            set_material(material, light, eye, part.surface, part.emission, coverage);
            // Restore the renderer's stack before the next independent draw.
            unsafe {
                get_internal_gl().quad_gl.push_model_matrix(matrix(part));
            }
            draw_mesh(&part.mesh);
            unsafe {
                get_internal_gl().quad_gl.pop_model_matrix();
            }
        };
        let mut translucent = Vec::new();
        for part in &model.parts {
            if part.blend {
                translucent.push((
                    matrix(part)
                        .transform_point3(part.center)
                        .distance_squared(eye),
                    part,
                ));
            } else {
                draw(part, opaque);
            }
        }
        translucent.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, part) in translucent {
            draw(part, blend);
        }
        gl_use_default_material();
    }
}

fn set_material(
    material: &Material,
    light: Vec3,
    eye: Vec3,
    surface: Vec4,
    emission: Vec3,
    coverage: f32,
) {
    material.set_uniform("Light", light);
    material.set_uniform("Eye", eye);
    material.set_uniform("Surface", surface);
    material.set_uniform("Emission", emission);
    material.set_uniform("Coverage", coverage);
    material.set_uniform("Opacity", 1.0_f32);
    material.set_uniform("VertexAlpha", 1.0_f32);
    gl_use_material(material);
}

impl Model {
    fn load(bytes: &[u8], clips: &[String], anchors: &[String]) -> Result<Self> {
        let file = Gltf::from_slice(bytes)?;
        let data = file.blob.as_deref().ok_or("expected binary GLB")?;
        if file.buffers().count() != 1
            || file.nodes().count() > 256
            || file.images().count() > 16
            || file.accessors().any(|a| a.count() > 600_000)
        {
            return Err("GLB exceeds rigid geometry limits".into());
        }
        let buffer =
            |b: buffer::Buffer<'_>| matches!(b.source(), buffer::Source::Bin).then_some(data);
        let mut textures = Vec::new();
        for image in file.images() {
            let image::Source::View { view, .. } = image.source() else {
                return Err("GLB images must be embedded".into());
            };
            let end = view
                .offset()
                .checked_add(view.length())
                .ok_or("GLB image overflow")?;
            let bytes = data
                .get(view.offset()..end)
                .ok_or("GLB image out of bounds")?;
            let image = Image::from_file_with_format(bytes, None)?;
            textures.push(Texture2D::from_image(&image));
        }
        let scene = file.default_scene().ok_or("GLB needs a default scene")?;
        let mut queue: Vec<_> = scene.nodes().map(|node| (None, node)).collect();
        let mut indices = [None; 256];
        let mut model = Self {
            parts: Vec::new(),
            nodes: Vec::new(),
            clips: Vec::new(),
            anchors: Vec::new(),
            minimum_y: f32::INFINITY,
            low: Vec3::splat(f32::INFINITY),
            high: Vec3::splat(f32::NEG_INFINITY),
        };
        let mut total = 0;
        while let Some((parent, node)) = queue.pop() {
            let node_index = model.nodes.len();
            if indices[node.index()].replace(node_index).is_some() || node.skin().is_some() {
                return Err("GLB must be a rigid scene tree".into());
            }
            let local = Mat4::from_cols_array_2d(&node.transform().matrix());
            let bind = parent.map_or(local, |p: usize| model.nodes[p].bind * local);
            if !bind.is_finite() || !bind.inverse().is_finite() {
                return Err("invalid GLB transform".into());
            }
            model.nodes.push(Node::new(parent, bind, node.transform()));
            queue.extend(node.children().map(|child| (Some(node_index), child)));
            let Some(mesh) = node.mesh() else {
                continue;
            };
            for part in mesh.primitives() {
                if part.mode() != Mode::Triangles || part.morph_targets().len() != 0 {
                    return Err("only rigid GLB triangles are supported".into());
                }
                let material = part.material();
                let pbr = material.pbr_metallic_roughness();
                let surface = vec4(
                    pbr.roughness_factor(),
                    pbr.metallic_factor(),
                    material.alpha_cutoff().unwrap_or(
                        if material.alpha_mode() == gltf::material::AlphaMode::Mask {
                            0.5
                        } else {
                            0.0
                        },
                    ),
                    0.0,
                );
                let emission = Vec3::from_array(material.emissive_factor());
                let blend = material.alpha_mode() == gltf::material::AlphaMode::Blend;
                let base = Vec4::from_array(pbr.base_color_factor());
                let info = pbr.base_color_texture();
                let texture = info
                    .as_ref()
                    .map(|info| textures[info.texture().source().index()].clone());
                let reader = part.reader(buffer);
                let positions = reader.read_positions().ok_or("GLB position data missing")?;
                let n = positions.len();
                let mut normal = reader.read_normals();
                let mut uv = reader
                    .read_tex_coords(info.map_or(0, |info| info.tex_coord()))
                    .map(|v| v.into_f32());
                let mut colors = reader.read_colors(0).map(|v| v.into_rgba_f32());
                let mut vertices = Vec::with_capacity(n);
                let mut low = Vec3::splat(f32::INFINITY);
                let mut high = Vec3::splat(f32::NEG_INFINITY);
                for position in positions {
                    let position = Vec3::from_array(position);
                    let normal = normal
                        .as_mut()
                        .and_then(Iterator::next)
                        .map_or(Vec3::Y, Vec3::from_array);
                    let normal = normal.try_normalize().unwrap_or(Vec3::Y);
                    let uv = uv
                        .as_mut()
                        .and_then(Iterator::next)
                        .map_or(Vec2::ZERO, Vec2::from_array);
                    let tint = colors
                        .as_mut()
                        .and_then(Iterator::next)
                        .map_or(base, |v| base * Vec4::from_array(v));
                    if !position.is_finite() || !uv.is_finite() || !tint.is_finite() {
                        return Err("non-finite GLB vertex".into());
                    }
                    low = low.min(position);
                    high = high.max(position);
                    let world = bind.transform_point3(position);
                    model.minimum_y = model.minimum_y.min(world.y);
                    model.low = model.low.min(world);
                    model.high = model.high.max(world);
                    let mut vertex =
                        Vertex::new2(position, uv, Color::new(tint.x, tint.y, tint.z, tint.w));
                    vertex.normal = normal.extend(0.0);
                    vertices.push(vertex);
                }
                let indices: Vec<_> = reader.read_indices().map_or_else(
                    || (0..n as u32).collect(),
                    |indices| indices.into_u32().collect(),
                );
                total += indices.len();
                if total > 1_500_000 || !indices.len().is_multiple_of(3) {
                    return Err("invalid GLB triangle index budget".into());
                }
                for batch in indices.chunks(BATCH) {
                    let mut mesh = Mesh {
                        vertices: Vec::new(),
                        indices: Vec::with_capacity(batch.len()),
                        texture: texture.clone(),
                    };
                    let mut mapped = HashMap::new();
                    for &index in batch {
                        let vertex = *vertices
                            .get(index as usize)
                            .ok_or("GLB vertex index out of bounds")?;
                        let index = *mapped.entry(index).or_insert_with(|| {
                            let index = mesh.vertices.len() as u16;
                            mesh.vertices.push(vertex);
                            index
                        });
                        mesh.indices.push(index);
                    }
                    model.parts.push(Part {
                        node: node_index,
                        mesh,
                        surface,
                        emission,
                        blend,
                        center: (low + high) * 0.5,
                    });
                }
            }
        }
        if model.parts.is_empty() {
            return Err("GLB scene has no triangles".into());
        }
        model.clips = clips
            .iter()
            .map(|name| Clip::load(&file, name, &indices))
            .collect::<Result<_>>()?;
        for anchor in anchors {
            let fields: Vec<_> = anchor.split(',').collect();
            let [name, x, y, z] = fields.as_slice() else {
                return Err("expected a node name and anchor offset".into());
            };
            let node = file
                .nodes()
                .find(|node| node.name() == Some(*name))
                .and_then(|node| indices[node.index()])
                .ok_or("anchor node absent from GLB")?;
            let offset = vec3(x.parse()?, y.parse()?, z.parse()?);
            if !offset.is_finite() {
                return Err("invalid anchor offset".into());
            }
            model.anchors.push((node, offset));
        }
        Ok(model)
    }
}
