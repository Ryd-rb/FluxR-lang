use super::request;
use crate::structured::{Module, Value};
use macroquad::audio::{
    PlaySoundParams, Sound, load_sound_from_bytes, play_sound, set_sound_volume,
};
use std::{error::Error, fs, io, path::Path};

pub struct Mix {
    levels: Vec<f32>,
    cues: Vec<(usize, f32)>,
}

impl Mix {
    pub fn project(module: &Module, export: &str, args: &[Value]) -> Result<Self, Box<dyn Error>> {
        Self::from_value(&request(module, export, args)?)
    }

    pub(super) fn from_value(value: &Value) -> Result<Self, Box<dyn Error>> {
        let Value::Record(fields) = value else {
            return Err("expected an audio mix record".into());
        };
        let [Value::List(levels), Value::List(cues)] = fields.as_ref() else {
            return Err("expected loop levels and audio cues".into());
        };
        let level = |value: &Value| match value {
            Value::I64(value) if (0..=1000).contains(value) => Ok(*value as f32 / 1000.0),
            _ => Err("audio level must be between 0 and 1000"),
        };
        let levels = levels.iter().map(level).collect::<Result<_, _>>()?;
        let cues = cues
            .iter()
            .map(|cue| {
                if let Value::Record(fields) = cue
                    && let [Value::I64(asset), gain] = fields.as_ref()
                {
                    return Ok((usize::try_from(*asset)?, level(gain)?));
                }
                Err::<_, Box<dyn Error>>("expected an audio cue record".into())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { levels, cues })
    }

    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    pub fn cues(&self) -> &[(usize, f32)] {
        &self.cues
    }

    pub fn replace_levels(&mut self, next: Self) {
        self.levels = next.levels;
        self.cues.extend(next.cues);
    }
}

pub struct Mixer {
    loops: Vec<Sound>,
    cues: Vec<Sound>,
    levels: Vec<f32>,
    targets: Vec<f32>,
}

impl Mixer {
    pub async fn load(catalog: &Path, assets: &Path) -> Result<Option<Self>, Box<dyn Error>> {
        let catalog = match fs::read_to_string(catalog) {
            Ok(catalog) => catalog,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut loops = Vec::new();
        let mut cues = Vec::new();
        for row in catalog.lines().filter(|row| !row.trim().is_empty()) {
            let (kind, filename) = row.split_once('\t').ok_or("invalid audio catalog row")?;
            if !filename.ends_with(".ogg")
                || !filename
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            {
                return Err("audio catalog expects a filename".into());
            }
            let sounds = match kind {
                "loop" => &mut loops,
                "cue" => &mut cues,
                _ => return Err("unknown audio catalog row type".into()),
            };
            let bytes = fs::read(assets.join(filename))?;
            sounds.push(load_sound_from_bytes(&bytes).await?);
        }
        let levels = vec![0.0; loops.len()];
        for sound in &loops {
            play_sound(
                sound,
                PlaySoundParams {
                    looped: true,
                    volume: 0.0,
                },
            );
        }
        Ok(Some(Self {
            loops,
            cues,
            targets: levels.clone(),
            levels,
        }))
    }

    pub fn apply(&mut self, mix: Mix) -> Result<(), &'static str> {
        if mix.levels.len() != self.loops.len()
            || mix.cues.iter().any(|(asset, _)| *asset >= self.cues.len())
        {
            return Err("audio mix does not match the catalog");
        }
        self.targets = mix.levels;
        for (asset, volume) in mix.cues {
            play_sound(
                &self.cues[asset],
                PlaySoundParams {
                    looped: false,
                    volume,
                },
            );
        }
        Ok(())
    }

    pub fn advance(&mut self, dt: f32) {
        for ((sound, level), &target) in self.loops.iter().zip(&mut self.levels).zip(&self.targets)
        {
            let step = (target - *level).clamp(-dt, dt);
            if step != 0.0 {
                *level += step;
                set_sound_volume(sound, *level);
            }
        }
    }
}
