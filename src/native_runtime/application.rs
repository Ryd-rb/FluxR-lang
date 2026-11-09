use super::{
    Mix, Mixer, WindowRequest,
    application_peer::{Peer, PeerRequest},
    application_service::Service,
    canvas,
    geometry::Geometry,
    heightfield::Heightfield,
    inspection, mesh,
};
use crate::structured::{Invocation, Limits, Module, Value, Workspace};
use macroquad::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::{
    error::Error,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const GEOMETRY_LIMITS: Limits = Limits {
    instructions: 80_000_000,
    cells: 12_000_000,
    frames: 64,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    program: PathBuf,
    titles: PathBuf,
    text: TextCatalog,
    assets: PathBuf,
    font: Option<PathBuf>,
    audio: Option<PathBuf>,
    meshes: Option<PathBuf>,
    field_textures: Option<Vec<PathBuf>>,
    #[serde(default)]
    keyboard: bool,
    #[serde(default)]
    clock: bool,
    #[serde(default)]
    scene_geometry: bool,
    frame_instructions: Option<u64>,
    frame_cells: Option<usize>,
    service: Option<PathBuf>,
    peer_key_env: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TextCatalog {
    File(PathBuf),
    Sources(Vec<TextSource>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextSource {
    path: PathBuf,
    #[serde(default)]
    columns: Vec<usize>,
}

pub(super) fn child(root: &Path, path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("application paths must be relative to their manifest".into());
    }
    Ok(root.join(path))
}

pub(super) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds the application file size limit", path.display()).into());
    }
    Ok(bytes)
}

pub struct Application {
    module: Module,
    window: WindowRequest,
    labels: Vec<String>,
    font: Option<Vec<u8>>,
    audio: Option<PathBuf>,
    assets: PathBuf,
    meshes: Vec<mesh::Source>,
    field_textures: Option<Vec<PathBuf>>,
    keyboard: bool,
    clock: bool,
    scene_geometry: bool,
    frame_limits: Limits,
    service: Option<Service>,
    peer_key_env: Option<String>,
}

impl Application {
    pub fn load(path: &Path, load_module: impl FnOnce(&Path) -> Result<Module>) -> Result<Self> {
        let manifest: Manifest = serde_json::from_slice(&read(path, 16384)?)?;
        let mut frame_limits = Limits::default();
        frame_limits.instructions = manifest
            .frame_instructions
            .unwrap_or(frame_limits.instructions);
        frame_limits.cells = manifest.frame_cells.unwrap_or(frame_limits.cells);
        if !(1..=1_000_000).contains(&frame_limits.instructions)
            || !(1..=1_000_000).contains(&frame_limits.cells)
        {
            return Err("application frame budget is outside host limits".into());
        }
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let module = load_module(&child(root, &manifest.program)?)?;
        let lines = |path: &Path| -> Result<Vec<String>> {
            let text = String::from_utf8(read(&child(root, path)?, 262144)?)?;
            Ok(text.lines().map(str::to_owned).collect())
        };
        let titles = lines(&manifest.titles)?;
        let refs = titles.iter().map(String::as_str).collect::<Vec<_>>();
        let window = WindowRequest::project(&module, "open_window", &refs)?;
        let audio = manifest.audio.map(|path| child(root, &path)).transpose()?;
        if window.audio_enabled() && audio.is_none() {
            return Err("application requested audio without a catalog".into());
        }
        let assets = child(root, &manifest.assets)?;
        let labels = match manifest.text {
            TextCatalog::File(path) => lines(&path)?,
            TextCatalog::Sources(sources) => {
                let mut labels = Vec::new();
                for source in sources {
                    for line in lines(&source.path)? {
                        if source.columns.is_empty() {
                            labels.push(line);
                        } else {
                            let fields = line.split('\t').collect::<Vec<_>>();
                            for column in &source.columns {
                                labels.push(
                                    fields
                                        .get(*column)
                                        .ok_or("missing text column")?
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
                labels
            }
        };
        let meshes = manifest
            .meshes
            .map(|path| {
                lines(&path)?
                    .iter()
                    .map(|row| {
                        let mut fields = row.split('\t');
                        let path = child(&assets, Path::new(fields.next().unwrap_or_default()))?;
                        let mut clips = Vec::new();
                        let mut anchors = Vec::new();
                        for field in fields {
                            if let Some(anchor) = field.strip_prefix("@anchor=") {
                                anchors.push(anchor.to_owned());
                            } else {
                                clips.push(field.to_owned());
                            }
                        }
                        if clips.len() > 32
                            || clips.iter().any(String::is_empty)
                            || anchors.len() > 2
                        {
                            return Err("invalid mesh clip catalog row".into());
                        }
                        Ok(mesh::Source {
                            path,
                            clips,
                            anchors,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let field_textures = manifest
            .field_textures
            .map(|paths| {
                if paths.len() > 16 {
                    return Err("too many heightfield textures".into());
                }
                paths
                    .iter()
                    .map(|path| child(&assets, path))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        Ok(Self {
            module,
            window,
            labels,
            font: manifest
                .font
                .map(|path| read(&child(root, &path)?, 4194304))
                .transpose()?,
            audio,
            assets,
            meshes,
            field_textures,
            keyboard: manifest.keyboard,
            clock: manifest.clock,
            scene_geometry: manifest.scene_geometry,
            frame_limits,
            service: manifest
                .service
                .map(|path| Service::load(&child(root, &path)?))
                .transpose()?,
            peer_key_env: manifest.peer_key_env,
        })
    }

    pub fn run(self, args: Vec<Value>, captures: Option<&Path>) -> Result<()> {
        self.run_with_peer(args, captures, None)
    }

    pub fn run_with_peer(
        self,
        args: Vec<Value>,
        captures: Option<&Path>,
        peer: Option<PeerRequest>,
    ) -> Result<()> {
        let mut program = Program {
            module: self.module,
            workspace: Workspace::default(),
            keyboard: self.keyboard,
            clock: self.clock,
            scene_geometry: self.scene_geometry,
            samples: self.window.samples,
            frame_limits: self.frame_limits,
        };
        let args = Value::List(Arc::new(args));
        let peer = peer
            .map(|request| {
                let key = self
                    .peer_key_env
                    .as_deref()
                    .ok_or("application does not declare a peer effect")?;
                Peer::start(program.module.clone(), request, key, args.clone())
            })
            .transpose()?;
        let state = if let Some(peer) = &peer {
            program.invoke("app_peer_scene", &[Value::I64(peer.slot), args.clone()])?
        } else {
            program.invoke("app_init", std::slice::from_ref(&args))?
        };
        let fields = if self.field_textures.is_some() && !self.scene_geometry {
            let value = program.invoke_with("app_fields", &[args], GEOMETRY_LIMITS)?;
            let Value::List(fields) = value else {
                return Err("expected an application heightfield list".into());
            };
            if fields.len() > 8 {
                return Err("too many application heightfields".into());
            }
            Some(fields)
        } else {
            None
        };
        let inspector = captures.map(inspection::Inspection::open).transpose()?;
        let audio = self.audio.filter(|_| self.window.audio_enabled());
        self.window.open(32768, async move {
            let result = async {
                let font = self
                    .font
                    .as_deref()
                    .map(load_ttf_font_from_bytes)
                    .transpose()?;
                let mut mixer = match audio {
                    Some(path) => Some(
                        Mixer::load(&path, &self.assets)
                            .await?
                            .ok_or("audio catalog is missing")?,
                    ),
                    None => None,
                };
                let textures = self
                    .field_textures
                    .unwrap_or_default()
                    .iter()
                    .map(|path| -> Result<_> {
                        let bytes = read(path, 8 * 1024 * 1024)?;
                        let image = Image::from_file_with_format(&bytes, None)?;
                        let texture = Texture2D::from_image(&image);
                        // The terrain shader samples this texture periodically.
                        Ok(texture)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut geometry = Geometry::default();
                if let Some(fields) = fields {
                    for field in fields.iter() {
                        geometry.fields.push(Heightfield::load(field, &textures)?);
                    }
                }
                let mut meshes = mesh::Catalog::new(self.meshes, geometry, textures);
                run_frames(
                    program,
                    state,
                    &self.labels,
                    font.as_ref(),
                    mixer.as_mut(),
                    &mut meshes,
                    Connections {
                        inspector,
                        service: if peer.is_none() { self.service } else { None },
                        peer,
                    },
                )
                .await
            }
            .await;
            if let Err(error) = result {
                eprintln!("application failed: {error}");
            }
            macroquad::miniquad::window::order_quit();
        });
        Ok(())
    }
}

struct Program {
    module: Module,
    workspace: Workspace,
    keyboard: bool,
    clock: bool,
    scene_geometry: bool,
    samples: i32,
    frame_limits: Limits,
}

impl Program {
    fn invoke(&mut self, export: &str, arguments: &[Value]) -> Result<Value> {
        self.invoke_with(export, arguments, Limits::default())
    }

    fn invoke_with(&mut self, export: &str, arguments: &[Value], limits: Limits) -> Result<Value> {
        Ok(self.module.run_with(
            Invocation {
                export,
                arguments,
                inputs: &[],
                limits,
            },
            &mut self.workspace,
        )?)
    }
}

struct Connections {
    inspector: Option<inspection::Inspection>,
    service: Option<Service>,
    peer: Option<Peer>,
}

async fn run_frames(
    mut program: Program,
    mut state: Value,
    labels: &[String],
    font: Option<&Font>,
    mut mixer: Option<&mut Mixer>,
    meshes: &mut mesh::Catalog,
    connections: Connections,
) -> Result<()> {
    let Connections {
        inspector,
        mut service,
        peer,
    } = connections;
    let mut previous_mouse = mouse_position();
    let mut index = 0_u64;
    let mut geometry_key = None;
    let mut timing = inspection::FrameTiming::default();
    loop {
        let request = inspector.as_ref().and_then(|i| i.requests.try_recv().ok());
        let (id, frame) = match request {
            Some(inspection::Request::Frame(id, frame)) => (Some(id), frame),
            Some(inspection::Request::Close(id)) => {
                inspection::result(id, json!({"closed":true}))?;
                return Ok(());
            }
            None => (None, inspection::Frame::default()),
        };
        if let Some([w, h]) = frame.resize {
            request_new_screen_size(w as f32, h as f32);
            // Allow the platform to deliver its resize before sampling the input dimensions.
            for _ in 0..4 {
                next_frame().await;
                if (screen_width() - w as f32).abs() < 1.0
                    && (screen_height() - h as f32).abs() < 1.0
                {
                    break;
                }
            }
        }
        let dpi = screen_dpi_scale();
        let (w, h) = (
            screen_width().round() as i64,
            screen_height().round() as i64,
        );
        let frame_seconds = get_frame_time();
        let profile = timing.frame(frame.profile, frame_seconds);
        let dt = frame_seconds.clamp(0.0, 0.25);
        let native_mouse = mouse_position();
        let delta = (
            native_mouse.0 - previous_mouse.0,
            native_mouse.1 - previous_mouse.1,
        );
        previous_mouse = native_mouse;
        let mouse = frame
            .pointer
            .map_or(native_mouse, |[x, y]| (x as f32, y as f32));
        let [dx, dy] = frame.drag.unwrap_or([delta.0, delta.1]);
        let wheel = frame.wheel.unwrap_or_else(|| {
            let wheel = mouse_wheel().1;
            if cfg!(windows) { wheel / 120.0 } else { wheel }
        });
        let mut characters: Vec<_> = frame.text.chars().map(|c| Value::I64(c as i64)).collect();
        while characters.len() < 128 {
            let Some(c) = get_char_pressed() else {
                break;
            };
            characters.push(Value::I64(c as i64));
        }
        use Value::{Bool, F32, I64, List, Record};
        let mut fields = vec![
            I64(w),
            I64(h),
            F32(dpi.to_bits()),
            F32(dt.to_bits()),
            I64(mouse.0.round() as i64),
            I64(mouse.1.round() as i64),
            F32(dx.to_bits()),
            F32(dy.to_bits()),
            F32(wheel.to_bits()),
            Bool(frame.click || is_mouse_button_pressed(MouseButton::Left)),
            Bool(frame.drag.is_some() || is_mouse_button_down(MouseButton::Right)),
            Bool(is_key_pressed(KeyCode::Escape)),
            Bool(is_key_pressed(KeyCode::Backspace)),
            Bool(is_key_pressed(KeyCode::Enter)),
            List(Arc::new(characters)),
            I64(frame.action),
            I64(frame.arg),
        ];
        if program.keyboard {
            let mut pressed: Vec<i64> = get_keys_pressed()
                .into_iter()
                .map(|key| key as i64)
                .collect();
            pressed.extend(frame.pressed);
            pressed.sort_unstable();
            pressed.dedup();
            let mut held: Vec<i64> = get_keys_down().into_iter().map(|key| key as i64).collect();
            held.sort_unstable();
            fields.push(List(Arc::new(pressed.into_iter().map(I64).collect())));
            fields.push(List(Arc::new(held.into_iter().map(I64).collect())));
        }
        let now = (get_time() * 1000.0) as i64;
        if program.clock {
            fields.push(I64(now));
        }
        if let Some(service) = &mut service {
            let limits = program.frame_limits;
            service.pump(&mut state, now, &mut |export, arguments| {
                program.invoke_with(export, arguments, limits)
            })?;
        }
        let input = Record(fields.into());
        let (peer_report, peer_error) = if let Some(peer) = &peer {
            let (snapshot, error) = peer.snapshot();
            let report = if let Some((snapshot, report)) = snapshot {
                state = program.invoke("app_peer_view", &[state, snapshot, input.clone()])?;
                Some(report)
            } else {
                None
            };
            if error.is_some() {
                state = program.invoke("app_peer_failed", &[state])?;
            }
            (report, error)
        } else {
            (None, None)
        };
        let next = program
            .invoke_with("app_frame", &[state, input], program.frame_limits)
            .map_err(|error| format!("app_frame: {error}"))?;
        let Record(fields) = next else {
            return Err("expected an application frame".into());
        };
        let (next_state, draws, audio, marker, key) = match fields.as_ref() {
            [state, List(draws), audio, I64(marker)] if !program.scene_geometry => {
                (state, draws, audio, marker, None)
            }
            [state, List(draws), audio, I64(marker), List(key)] if program.scene_geometry => {
                (state, draws, audio, marker, Some(key))
            }
            _ => {
                return Err(
                    "expected state, draws, audio, frame marker and configured geometry key".into(),
                );
            }
        };
        if let Some(key) = key {
            if key.len() > 16 || !key.iter().all(|value| matches!(value, I64(_))) {
                return Err("geometry key must contain up to 16 integers".into());
            }
            if geometry_key.as_ref() != Some(key) {
                let scene = program.invoke_with(
                    "app_geometry",
                    &[List(key.clone())],
                    Limits {
                        cells: 18_000_000,
                        ..GEOMETRY_LIMITS
                    },
                )?;
                meshes.geometry = Geometry::load(&scene, &meshes.textures)?;
                geometry_key = Some(key.clone());
            }
        }
        state = next_state.clone();
        if let Some(peer) = &peer {
            let result = program.invoke("app_peer_order", &[state])?;
            let Record(result) = result else {
                return Err("expected an application peer order".into());
            };
            let [next, Value::List(orders)] = result.as_ref() else {
                return Err("expected application state and optional peer order".into());
            };
            state = next.clone();
            if let [I64(order)] = orders.as_slice() {
                if !peer.submit(*order) {
                    state = program.invoke("app_peer_rejected", &[state])?;
                }
            } else if !orders.is_empty() {
                return Err("application queued more than one peer order".into());
            }
        }
        let mix = Mix::from_value(audio)?;
        let levels = id.as_ref().map(|_| mix.levels().to_vec());
        let cues = id.as_ref().map(|_| mix.cues().to_vec());
        if let Some(mixer) = mixer.as_deref_mut() {
            mixer.apply(mix)?;
            mixer.advance(dt);
        }
        let mut contacts = id.as_ref().map(|_| Vec::new());
        let (mesh_draws, camera) =
            canvas::render(draws, labels, font, dpi, meshes, contacts.as_mut())?;
        index += 1;
        if let (Some(inspector), Some(id)) = (&inspector, id) {
            let path = frame
                .capture
                .map(|name| inspector.root.join(format!("{name}.png")));
            if path.as_ref().is_some_and(|path| path.exists()) {
                inspection::fail(id, "capture already exists")?;
            } else {
                if let Some(path) = &path {
                    let path = path.to_str().ok_or("capture path must be UTF-8")?;
                    get_screen_data().export_png(path);
                }
                let (pixel_w, pixel_h) = macroquad::miniquad::window::screen_size();
                let service = service
                    .as_ref()
                    .map(|service| {
                        service.inspect(&state, now, &mut |export, arguments| {
                            program.invoke(export, arguments)
                        })
                    })
                    .transpose()?;
                let peer_scene = peer
                    .as_ref()
                    .map(|_| {
                        super::application_peer::numbers(
                            &program.invoke("app_peer_inspect", std::slice::from_ref(&state))?,
                        )
                    })
                    .transpose()?;
                inspection::result(
                    id,
                    json!({
                        "frame": index, "logical": [w, h], "framebuffer": [pixel_w as u32, pixel_h as u32],
                        "dpi": dpi, "clock_ms": now, "profile": profile,
                        "marker": marker, "audio_loaded": mixer.is_some(), "audio_targets": levels,
                    "audio_cues": cues, "mesh_draws": mesh_draws, "contacts": contacts,
                        "capture": path, "camera": camera, "requested_samples": program.samples,
                        "service": service,
                        "peer": peer_report, "peer_error": peer_error, "peer_scene": peer_scene,
                        "geometry_key": geometry_key.as_ref().map(|key| key.iter().filter_map(|value| {
                            if let I64(value) = value { Some(*value) } else { None }
                        }).collect::<Vec<_>>()),
                    }),
                )?;
            }
        }
        next_frame().await;
    }
}
