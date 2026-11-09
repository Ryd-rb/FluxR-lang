//! Native callbacks driven by typed program requests.
use crate::structured::{Invocation, Limits, Module, Value};
use macroquad::{Window, conf::Conf, miniquad};
use std::{error::Error, future::Future};

mod animation;
mod application;
mod application_peer;
mod application_service;
mod audio;
mod canvas;
mod geometry;
mod heightfield;
mod inspection;
mod mesh;
mod swarm;
pub use application::Application;
pub use application_peer::PeerRequest;
pub use application_service::serve as application_service;
pub use audio::{Mix, Mixer};
pub use swarm::{Swarm, SwarmSample, Workpiece};

fn request(module: &Module, export: &str, arguments: &[Value]) -> Result<Value, crate::Error> {
    module.run(Invocation {
        export,
        inputs: &[],
        arguments,
        limits: Limits::default(),
    })
}

pub struct WindowRequest {
    title: String,
    width: i32,
    height: i32,
    samples: i32,
    audio: bool,
    high_dpi: bool,
}

impl WindowRequest {
    /// Project a window request. The first field indexes the host's title registry.
    pub fn project(module: &Module, export: &str, titles: &[&str]) -> Result<Self, Box<dyn Error>> {
        use Value::{Bool, I64, Record};
        let Record(fields) = request(module, export, &[])? else {
            return Err("expected a window request record".into());
        };
        let [
            I64(title),
            I64(width),
            I64(height),
            I64(samples),
            Bool(audio),
            options @ ..,
        ] = fields.as_ref()
        else {
            return Err("invalid window request fields".into());
        };
        let high_dpi = match options {
            [] => false,
            [Bool(enabled)] => *enabled,
            _ => return Err("invalid window DPI option".into()),
        };
        let title = titles
            .get(usize::try_from(*title)?)
            .filter(|title| !title.is_empty() && title.len() <= 256 && !title.contains('\0'))
            .ok_or("invalid window title")?;
        if !(256..=8192).contains(width)
            || !(256..=8192).contains(height)
            || !matches!(samples, 1 | 2 | 4 | 8 | 16)
        {
            return Err("window request exceeds supported dimensions or sample counts".into());
        }
        Ok(Self {
            title: (*title).to_owned(),
            width: *width as i32,
            height: *height as i32,
            samples: *samples as i32,
            audio: *audio,
            high_dpi,
        })
    }

    pub fn audio_enabled(&self) -> bool {
        self.audio
    }

    pub fn open(self, draw_capacity: usize, application: impl Future<Output = ()> + 'static) {
        let width = self.width;
        let height = self.height;
        let high_dpi = self.high_dpi;
        Window::from_config(
            Conf {
                miniquad_conf: miniquad::conf::Conf {
                    window_title: self.title,
                    window_width: self.width,
                    window_height: self.height,
                    sample_count: self.samples,
                    high_dpi: self.high_dpi,
                    ..Default::default()
                },
                draw_call_vertex_capacity: draw_capacity,
                draw_call_index_capacity: draw_capacity,
                ..Default::default()
            },
            async move {
                // Win32 creates the initial client rectangle in physical pixels.
                // Restore the requested logical size once the monitor scale is known.
                if cfg!(windows) && high_dpi && macroquad::window::screen_dpi_scale() != 1.0 {
                    macroquad::window::request_new_screen_size(width as f32, height as f32);
                    macroquad::window::next_frame().await;
                }
                application.await;
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_valid_window_and_rejects_invalid_requests() -> Result<(), Box<dyn Error>> {
        let program = Module::compile_compact(
            "v2 (rec Settings title:i64 w:i64 h:i64 samples:i64 audio:bool) \
             (pub open () Settings () (ret (new Settings 0 960 600 4 true))) \
             (pub invalid () Settings () (ret (new Settings 0 10 600 4 false)))",
        )?;
        let window = WindowRequest::project(&program, "open", &["Test"])?;
        assert_eq!((window.width, window.height, window.samples), (960, 600, 4));
        assert!(window.audio_enabled());
        assert!(!window.high_dpi);
        assert!(WindowRequest::project(&program, "open", &[]).is_err());
        assert!(WindowRequest::project(&program, "invalid", &["Test"]).is_err());
        Ok(())
    }

    #[test]
    fn projects_optional_native_pixel_density() -> Result<(), Box<dyn Error>> {
        let program = Module::compile_compact(
            "v2 (rec Settings title:i64 w:i64 h:i64 samples:i64 audio:bool dpi:bool) \
             (pub open () Settings () (ret (new Settings 0 960 600 4 true true)))",
        )?;
        assert!(WindowRequest::project(&program, "open", &["Test"])?.high_dpi);
        Ok(())
    }
}
