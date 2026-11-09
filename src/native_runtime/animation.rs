use gltf::{
    Gltf,
    animation::{Interpolation, util::ReadOutputs},
    buffer,
};
use macroquad::prelude::*;
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
pub(super) struct Node {
    pub parent: Option<usize>,
    pub bind: Mat4,
    pose: Pose,
}

#[derive(Clone, Copy)]
struct Pose {
    scale: Vec3,
    rotation: Quat,
    translation: Vec3,
}

impl Node {
    pub fn new(parent: Option<usize>, bind: Mat4, local: gltf::scene::Transform) -> Self {
        let (translation, rotation, scale) = local.decomposed();
        Self {
            parent,
            bind,
            pose: Pose {
                scale: Vec3::from_array(scale),
                rotation: Quat::from_array(rotation),
                translation: Vec3::from_array(translation),
            },
        }
    }
}

enum Keys {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

struct Channel {
    node: usize,
    times: Vec<f32>,
    keys: Keys,
    step: bool,
}

pub(super) struct Clip {
    start: f32,
    end: f32,
    channels: Vec<Channel>,
}

#[derive(Clone, Copy)]
pub(super) enum Sample {
    Fraction(f32),
    Seconds(f32, bool),
}

impl Clip {
    pub fn load(file: &Gltf, name: &str, indices: &[Option<usize>; 256]) -> Result<Self> {
        let animation = file
            .animations()
            .find(|clip| clip.name() == Some(name))
            .ok_or_else(|| format!("GLB clip {name:?} missing"))?;
        let mut clip = Self {
            start: f32::INFINITY,
            end: 0.0,
            channels: Vec::new(),
        };
        let mut budget = 200_000usize;
        for channel in animation.channels() {
            let node = indices[channel.target().node().index()]
                .ok_or("animation target is outside the default scene")?;
            let interpolation = channel.sampler().interpolation();
            if interpolation == Interpolation::CubicSpline {
                return Err("rigid node clips require linear or step keyframes".into());
            }
            let reader = channel.reader(|b| match b.source() {
                buffer::Source::Bin => file.blob.as_deref(),
                buffer::Source::Uri(_) => None,
            });
            let times = reader.read_inputs().ok_or("clip times missing")?;
            budget = budget
                .checked_sub(times.len())
                .ok_or("clip key budget exceeded")?;
            let times: Vec<_> = times.collect();
            if times.is_empty()
                || times[0] < 0.0
                || times.iter().any(|t| !t.is_finite())
                || times.windows(2).any(|t| t[0] >= t[1])
            {
                return Err("invalid clip key times".into());
            }
            let keys = match reader.read_outputs().ok_or("clip values missing")? {
                ReadOutputs::Translations(keys) => {
                    Keys::Translation(keys.map(Vec3::from_array).collect())
                }
                ReadOutputs::Scales(keys) => Keys::Scale(keys.map(Vec3::from_array).collect()),
                ReadOutputs::Rotations(keys) => {
                    Keys::Rotation(keys.into_f32().map(Quat::from_array).collect())
                }
                _ => return Err("only rigid node transforms are supported".into()),
            };
            let valid = match &keys {
                Keys::Translation(keys) | Keys::Scale(keys) => {
                    keys.len() == times.len() && keys.iter().all(|key| key.is_finite())
                }
                Keys::Rotation(keys) => {
                    keys.len() == times.len() && keys.iter().all(|key| key.is_normalized())
                }
            };
            if !valid {
                return Err("invalid clip key values".into());
            }
            clip.start = clip.start.min(times[0]);
            clip.end = clip.end.max(times[times.len() - 1]);
            clip.channels.push(Channel {
                node,
                times,
                keys,
                step: interpolation == Interpolation::Step,
            });
        }
        if clip.channels.is_empty() {
            return Err("empty GLB clip".into());
        }
        Ok(clip)
    }

    pub fn matrices(&self, nodes: &[Node], sample: Sample) -> Vec<Mat4> {
        let span = self.end - self.start;
        let time = self.start
            + match sample {
                Sample::Fraction(t) => t * span,
                Sample::Seconds(t, true) if span > 0.0 => t.rem_euclid(span),
                Sample::Seconds(t, _) => t.min(span),
            };
        let mut poses: Vec<_> = nodes.iter().map(|node| node.pose).collect();
        for channel in &self.channels {
            let end = channel.times.len() - 1;
            let right = channel.times.partition_point(|key| *key <= time).min(end);
            let left = right.saturating_sub(1);
            let (a, b) = (channel.times[left], channel.times[right]);
            let t = if a == b {
                0.0
            } else {
                ((time - a) / (b - a)).clamp(0.0, 1.0)
            };
            let t = if channel.step {
                (t >= 1.0) as u8 as f32
            } else {
                t
            };
            let pose = &mut poses[channel.node];
            match &channel.keys {
                Keys::Translation(keys) => pose.translation = keys[left].lerp(keys[right], t),
                Keys::Rotation(keys) => pose.rotation = keys[left].slerp(keys[right], t),
                Keys::Scale(keys) => pose.scale = keys[left].lerp(keys[right], t),
            }
        }
        let mut world = Vec::with_capacity(nodes.len());
        for (node, pose) in nodes.iter().zip(poses) {
            let local =
                Mat4::from_scale_rotation_translation(pose.scale, pose.rotation, pose.translation);
            world.push(node.parent.map_or(local, |parent| world[parent] * local));
        }
        world
    }
}
