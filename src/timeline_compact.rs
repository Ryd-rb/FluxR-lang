use super::*;
use std::fmt::Write;

impl Program {
    /// Migrate any released timeline to v2 source. The portable encoding stays v1.
    pub fn compact_text(&self) -> String {
        let mut out = format!("v2 {}", self.duration);
        let mut i = 0;
        while i < self.tracks.len() {
            let track = &self.tracks[i];
            let (object, field) = track.address;
            let run = |offset: usize| {
                let other = self.tracks.get(i + offset)?;
                (u16::try_from(usize::from(object) + offset).ok()? == other.address.0
                    && field == other.address.1)
                    .then(|| other.translation_from(track))
                    .flatten()
            };
            let mut count = 1;
            if let Some(step) = run(1) {
                count = 2;
                while run(count) == Some(step * count as i64) {
                    count += 1;
                }
                write!(
                    out,
                    "\n+ {object} {count} {field} {} {step}",
                    track.curve as u8
                )
                .unwrap();
            } else {
                write!(out, "\n{object} {field} {}", track.curve as u8).unwrap();
            }
            for &(time, value) in &*track.keys {
                write!(out, " {time}:{value}").unwrap();
            }
            i += count;
        }
        out
    }
}

pub(super) fn parse<'a>(
    duration: u32,
    lines: impl Iterator<Item = (usize, &'a str)>,
) -> Result<Program, Error> {
    let mut tracks = Vec::new();
    let mut keys = 0;
    for (index, line) in lines {
        add(line, &mut tracks, &mut keys)
            .map_err(|error| Error(format!("line {}: {error}", index + 1)))?;
    }
    Program::compile((VERSION, duration, tracks))
}

fn add(line: &str, tracks: &mut Vec<TrackSource>, keys: &mut usize) -> Result<(), Error> {
    let mut parts = line.split_whitespace();
    let head = parts.next();
    let (object, count, field, curve, step) = if head == Some("+") {
        let object = number(parts.next(), "object")?;
        let count = number(parts.next(), "track run length")?;
        let field = number(parts.next(), "field")?;
        let curve = number(parts.next(), "curve")?;
        let step = number(parts.next(), "track value step")?;
        (object, count, field, curve, step)
    } else {
        (
            number(head, "object")?,
            1,
            number(parts.next(), "field")?,
            number(parts.next(), "curve")?,
            0,
        )
    };
    let object: u16 = object;
    let step: i64 = step;
    if !(1..=MAX_TRACKS - tracks.len()).contains(&count)
        || object.checked_add((count - 1) as u16).is_none()
    {
        return Err(Error("track run exceeds address or track budget".into()));
    }
    let mut base = Vec::new();
    for pair in parts {
        *keys += count;
        if *keys > MAX_KEYS {
            return Err(Error("track run exceeds key budget".into()));
        }
        base.push(text_key(pair)?);
    }
    for i in 0..count {
        let shift = step.checked_mul(i as i64);
        let shifted = base
            .iter()
            .map(|&(t, value)| {
                let value = shift
                    .and_then(|delta| i64::from(value).checked_add(delta))
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or_else(|| Error("track run value outside i32".into()))?;
                Ok((t, value))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        tracks.push((object + i as u16, field, curve, shifted));
    }
    Ok(())
}
