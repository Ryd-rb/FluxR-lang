use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
use macroquad::{
    miniquad::{
        Bindings, BlendFactor, BlendState, BlendValue, BufferLayout, BufferSource, BufferType,
        BufferUsage, Comparison, Equation, PassAction, Pipeline, PipelineParams, ShaderMeta,
        ShaderSource, UniformBlockLayout, UniformDesc, UniformType, UniformsSource,
        VertexAttribute, VertexFormat,
    },
    prelude::*,
    window::get_internal_gl,
};

const CAPACITY: usize = 768;
const VERTEX: &str = r#"
#version 100
attribute vec3 SeededCorner;
uniform highp mat4 ViewProjection;
uniform highp vec4 NozzleA;
uniform highp vec4 NozzleB;
uniform highp vec4 Bed;
uniform highp vec4 Width;
uniform highp vec4 Depth;
uniform highp vec4 Height;
uniform highp vec4 Eye;
uniform highp vec4 Clock;
varying mediump vec2 corner;
varying mediump vec4 tint;

highp float noise(highp float n) {
    return fract(sin(n * 91.1537 + 13.721) * 19267.2539);
}

void main() {
    highp float id = SeededCorner.z;
    highp float seed = noise(id);
    highp float lifetime = 0.19 + 0.34 * noise(id + 117.0);
    highp float age = Clock.x - noise(id + 19.0) * 0.29;
    highp float time = Clock.x + Clock.z * 1.4;
    highp float beat = 0.56 + noise(id + 39.0) * 2.4;
    highp float cycle = max(0.0, age) / lifetime;
    cycle += sin(cycle * 1.73 + seed * 6.28) * 0.13;
    highp float phase = fract(max(0.0, cycle));
    highp float returned = smoothstep(0.04 * seed, 0.9 + 0.1 * seed, Clock.z);
    highp float run = phase * phase * (3.0 - 2.0 * phase) * (1.0 - returned);
    highp float front = clamp(1.0 - Clock.y + (noise(id + 90.0) - 0.5) * 0.41, 0.02, 0.98);
    highp vec3 target = Bed.xyz + Width.xyz * noise(id + 45.0) + Depth.xyz * front;
    target += Height.xyz * (0.25 + 0.75 * noise(id + 59.0));
    highp vec3 source = mix(NozzleA.xyz, NozzleB.xyz, step(0.5, noise(id + 228.0)));
    highp float arch = 4.0 * run * (1.0 - run);
    highp vec3 flow = target - source;
    highp vec3 point = mix(source, target, run);
    highp float roll = time * (12.0 + 8.0 * beat) + id * 3.43;
    highp float dart = sin(roll) + 0.39 * sin(roll * 2.71 + id);
    highp float veer = sin(roll * 0.67 + id * 0.37) + 0.35 * sin(roll * 3.79);
    point += Height.xyz * arch * (0.56 + 0.52 * noise(id + 249.0) + 0.27 * dart);
    point += Width.xyz * arch * (0.12 * veer + (noise(id + 294.0) - 0.5) * 0.18);
    point += Depth.xyz * arch * (0.09 * sin(roll * 1.29) + 0.045 * dart);
    highp vec3 sight = normalize(Eye.xyz - point);
    highp vec3 stretch = normalize(flow - sight * dot(flow, sight) + Width.xyz * 0.002);
    highp vec3 crosswise = cross(sight, stretch);
    highp float radius = Clock.w * mix(0.38, 1.1, noise(id + 322.0));
    highp float payload = step(0.70, seed);
    highp float fade = smoothstep(0.0, 0.09, age) * (1.0 - smoothstep(0.82, 1.0, Clock.z));
    fade *= smoothstep(0.0, 0.09, run);
    tint = vec4(mix(vec3(0.47, 0.77, 0.83), vec3(0.96, 0.76, 0.46), payload * (1.0 - smoothstep(0.0, 0.34, Clock.z))),
        fade * mix(0.45, 0.86, payload));
    corner = SeededCorner.xy;
    point += radius * (stretch * corner.x * 1.40 + crosswise * corner.y);
    gl_Position = ViewProjection * vec4(point, 1.0);
}
"#;

const FRAGMENT: &str = r#"
#version 100
precision mediump float;
varying mediump vec2 corner;
varying mediump vec4 tint;
void main() {
    float edge = dot(corner, corner);
    float alpha = tint.a * (1.0 - smoothstep(0.18, 1.0, edge));
    if (alpha < 0.02) discard;
    gl_FragColor = vec4(mix(tint.rgb, vec3(1.0, 0.94, 0.82), 0.33 * (1.0 - edge)), alpha);
}
"#;

pub struct SwarmSample {
    pub fill: f32,
    pub recall: f32,
    pub age: f32,
    pub density: f32,
    pub radius: f32,
}

#[repr(C)]
struct Uniforms {
    projection: [f32; 16],
    nozzle_a: [f32; 4],
    nozzle_b: [f32; 4],
    bed: [f32; 4],
    width: [f32; 4],
    depth: [f32; 4],
    height: [f32; 4],
    eye: [f32; 4],
    clock: [f32; 4],
}

pub struct Workpiece {
    pub bed: Vec3,
    pub width: Vec3,
    pub depth: Vec3,
    pub height: Vec3,
}

pub struct Swarm {
    pipeline: Pipeline,
    bindings: Bindings,
}

impl Swarm {
    pub fn new() -> Result<Self> {
        let mut vertices = Vec::with_capacity(CAPACITY * 4);
        let mut indices = Vec::with_capacity(CAPACITY * 6);
        for index in 0..CAPACITY {
            let first = vertices.len() as u16;
            for [x, y] in [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]] {
                vertices.push([x, y, index as f32]);
            }
            indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
        let gl = unsafe { get_internal_gl() };
        let ctx = gl.quad_context;
        let mut uniforms = vec![UniformDesc::new("ViewProjection", UniformType::Mat4)];
        uniforms.extend(
            [
                "NozzleA", "NozzleB", "Bed", "Width", "Depth", "Height", "Eye", "Clock",
            ]
            .into_iter()
            .map(|name| UniformDesc::new(name, UniformType::Float4)),
        );
        let shader = ctx
            .new_shader(
                ShaderSource::Glsl {
                    vertex: VERTEX,
                    fragment: FRAGMENT,
                },
                ShaderMeta {
                    uniforms: UniformBlockLayout { uniforms },
                    images: Vec::new(),
                },
            )
            .map_err(|error| format!("swarm shader: {error}"))?;
        let pipeline = ctx.new_pipeline(
            &[BufferLayout::default()],
            &[VertexAttribute::new("SeededCorner", VertexFormat::Float3)],
            shader,
            PipelineParams {
                depth_test: Comparison::LessOrEqual,
                depth_write: false,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
        );
        let vertex = ctx.new_buffer(
            BufferType::VertexBuffer,
            BufferUsage::Immutable,
            BufferSource::slice(&vertices),
        );
        let index_buffer = ctx.new_buffer(
            BufferType::IndexBuffer,
            BufferUsage::Immutable,
            BufferSource::slice(&indices),
        );
        Ok(Self {
            pipeline,
            bindings: Bindings {
                vertex_buffers: vec![vertex],
                index_buffer,
                images: Vec::new(),
            },
        })
    }

    pub fn draw(
        &self,
        sources: [Vec3; 2],
        bed: &Workpiece,
        eye: Vec3,
        projection: Mat4,
        clouds: &[SwarmSample],
    ) {
        if clouds.is_empty() {
            return;
        }
        let mut gl = unsafe { get_internal_gl() };
        gl.flush();
        let ctx = gl.quad_context;
        ctx.begin_default_pass(PassAction::Nothing);
        ctx.apply_pipeline(&self.pipeline);
        ctx.apply_bindings(&self.bindings);
        let vector = |v: Vec3| v.extend(0.0).to_array();
        for cloud in clouds {
            let uniforms = Uniforms {
                projection: projection.to_cols_array(),
                nozzle_a: vector(sources[0]),
                nozzle_b: vector(sources[1]),
                bed: vector(bed.bed),
                width: vector(bed.width),
                depth: vector(bed.depth),
                height: vector(bed.height),
                eye: vector(eye),
                clock: [
                    cloud.age,
                    cloud.fill,
                    cloud.recall,
                    cloud.radius * bed.width.length(),
                ],
            };
            ctx.apply_uniforms(UniformsSource::table(&uniforms));
            ctx.draw(0, cloud.density.clamp(0.0, CAPACITY as f32) as i32 * 6, 1);
        }
        ctx.end_render_pass();
    }
}
