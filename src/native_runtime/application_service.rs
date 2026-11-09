//! Optional process and capability-scoped MCP effects for a native application.
use crate::{
    authenticated_tcp::Endpoint,
    structured::{Invocation, Limits, Module, ModuleSlot, Value},
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, BufRead, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PACKET: usize = 16 * 1024;
const DOMAIN: &[u8] = b"fluxr/application-service/v1";
const IPC_TIMEOUT: Duration = Duration::from_secs(3);
const SOURCE_LIMIT: usize = 8192;
const NO_PROBE: i64 = i64::MIN;
const EFFECT: i64 = -1;
const EFFECT_RESULT: i64 = -2;
const INSPECT: i64 = -3;
const LOCAL_SCRIPT: i64 = -4;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    schema: Json,
    #[serde(default)]
    integers: Vec<String>,
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    script: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    namespace: String,
    config: PathBuf,
    config_env: String,
    instructions: String,
    catalog: PathBuf,
    tools: Vec<Tool>,
    fields: Vec<String>,
    flags: Vec<String>,
    errors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Runner {
    runner: String,
    executable: PathBuf,
    model: String,
    effort: String,
}

impl Runner {
    fn load(path: &Path, manifest: &Manifest) -> Result<Self> {
        let config = env::var_os(&manifest.config_env)
            .map(PathBuf::from)
            .map_or_else(|| super::application::child(path, &manifest.config), Ok)?;
        let value: Self = serde_json::from_slice(&super::application::read(&config, 16384)?)?;
        if value.runner != "codex"
            || !value.executable.is_absolute()
            || !value.executable.is_file()
            || value.model.is_empty()
            || !value
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || !matches!(
                value.effort.as_str(),
                "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
            )
        {
            return Err("invalid runner executable, model or effort".into());
        }
        Ok(value)
    }
}

pub(super) struct Service {
    path: PathBuf,
    manifest: Manifest,
    catalog: Vec<String>,
    run: Option<Run>,
    scripts: std::collections::HashMap<i64, Script>,
}

struct Script {
    source: String,
    module: ModuleSlot,
}

struct Call {
    tool: String,
    arguments: Json,
    deadline: Instant,
    reply: SyncSender<Json>,
}

struct Run {
    child: Option<Child>,
    directory: PathBuf,
    requests: Receiver<Call>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    slot: i64,
    epoch: i64,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn manifest(path: &Path) -> Result<Manifest> {
    let value: Manifest = serde_json::from_slice(&super::application::read(path, 65536)?)?;
    if value.tools.len() > 32
        || value.namespace.is_empty()
        || !value
            .namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("invalid application service manifest".into());
    }
    Ok(value)
}

fn packet(value: &Json) -> Result<[u8; PACKET]> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() >= PACKET {
        return Err("application service message exceeds the packet limit".into());
    }
    let mut frame = [0; PACKET];
    frame[..bytes.len()].copy_from_slice(&bytes);
    Ok(frame)
}

fn unpack(frame: &[u8; PACKET]) -> Result<Json> {
    let end = frame
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("unterminated packet")?;
    Ok(serde_json::from_slice(&frame[..end])?)
}

fn secret_hex(secret: &[u8; 32]) -> String {
    secret.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn secret_bytes(text: &str) -> Result<[u8; 32]> {
    let bytes = text.as_bytes().as_chunks::<2>();
    if bytes.0.len() != 32 || !bytes.1.is_empty() || !text.is_ascii() {
        return Err("invalid application service credential".into());
    }
    let mut secret = [0; 32];
    for (byte, pair) in secret.iter_mut().zip(bytes.0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(secret)
}

fn error(message: impl std::fmt::Display) -> Json {
    json!({"content":[{"type":"text","text":message.to_string()}],"isError":true})
}

fn receive(endpoint: Endpoint, secret: [u8; 32], stop: Arc<AtomicBool>, send: SyncSender<Call>) {
    while !stop.load(Ordering::Relaxed) {
        let _ = (|| -> Result<()> {
            let mut link =
                endpoint.open_with(&secret, DOMAIN, Duration::from_millis(200), IPC_TIMEOUT)?;
            let request = unpack(&link.exchange([0; PACKET])?)?;
            let (reply, result) = sync_channel(1);
            let deadline = Instant::now() + IPC_TIMEOUT;
            send.try_send(Call {
                tool: request["name"]
                    .as_str()
                    .ok_or("missing tool name")?
                    .to_owned(),
                arguments: request["arguments"].clone(),
                deadline,
                reply,
            })?;
            let response = result.recv_timeout(IPC_TIMEOUT)?;
            link.exchange(packet(&response).or_else(|err| packet(&error(err)))?)?;
            Ok(())
        })();
    }
}

impl Run {
    fn start(
        path: &Path,
        manifest: &Manifest,
        slot: i64,
        epoch: i64,
        prompt: &str,
    ) -> Result<Self> {
        let root = path.parent().ok_or("service manifest has no parent")?;
        let runner = Runner::load(root, manifest)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let mut secret = [0; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| "secure randomness unavailable")?;
        let directory = env::temp_dir().join(format!(
            "fluxr-service-{}-{}",
            std::process::id(),
            address.port()
        ));
        fs::create_dir(&directory)?;
        let (send, requests) = sync_channel(1);
        let mut run = Self {
            child: None,
            directory,
            requests,
            stop: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(180),
            slot,
            epoch,
        };
        let instructions = format!("{}\n\n{prompt}", manifest.instructions);
        if prompt.len() > 2048 || instructions.len() > 4096 {
            return Err("service prompt exceeds 4096 bytes".into());
        }
        let prefix = format!("mcp_servers.{}", manifest.namespace);
        let mut cmd = Command::new(runner.executable);
        cmd.args([
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--disable",
            "shell_tool",
            "--disable",
            "unified_exec",
            "--disable",
            "shell_snapshot",
            "--model",
            &runner.model,
        ])
        .arg("-c")
        .arg(format!("model_reasoning_effort={}", json!(runner.effort)))
        .args([
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "web_search=\"disabled\"",
        ])
        .arg("-c")
        .arg(format!("{prefix}.command={}", json!(env::current_exe()?)))
        .arg("-c")
        .arg(format!(
            "{prefix}.args={}",
            json!([
                "structured",
                "service",
                path.to_str().ok_or("service path must be UTF-8")?
            ])
        ))
        .arg("-c")
        .arg(format!("{prefix}.required=true"))
        .arg("-c")
        .arg(format!("{prefix}.default_tools_approval_mode=\"approve\""))
        .arg("-c")
        .arg(format!(
            "{prefix}.env_vars=[\"FLUXR_SERVICE_ADDRESS\",\"FLUXR_SERVICE_SECRET\"]"
        ))
        .env("FLUXR_SERVICE_ADDRESS", address.to_string())
        .env("FLUXR_SERVICE_SECRET", secret_hex(&secret))
        .current_dir(&run.directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
        if let Some(trace) = env::var_os("FLUXR_APP_SERVICE_TRACE") {
            let trace = PathBuf::from(trace);
            if !trace.is_absolute() {
                return Err("service trace must be an absolute file path".into());
            }
            cmd.arg("--json")
                .stdout(File::options().create(true).append(true).open(trace)?);
        }
        cmd.arg("-");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let stop = Arc::clone(&run.stop);
        std::thread::Builder::new()
            .name("application-service".into())
            .spawn(move || receive(Endpoint::Listen(listener), secret, stop, send))?;
        run.child = Some(cmd.spawn()?);
        run.child
            .as_mut()
            .unwrap()
            .stdin
            .take()
            .ok_or("runner stdin unavailable")?
            .write_all(instructions.as_bytes())?;
        Ok(run)
    }

    fn poll(&mut self) -> Option<i64> {
        let child = self.child.as_mut()?;
        let status = match child.try_wait() {
            Ok(Some(status)) => i64::from(status.success()),
            Ok(None) if Instant::now() < self.deadline => return None,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                -1
            }
        };
        self.child = None;
        self.stop.store(true, Ordering::Relaxed);
        Some(status)
    }
}

fn integers(values: impl IntoIterator<Item = i64>) -> Value {
    Value::List(Arc::new(values.into_iter().map(Value::I64).collect()))
}

fn text(value: &Value) -> Result<String> {
    let Value::List(chars) = value else {
        return Err("expected service text".into());
    };
    chars
        .iter()
        .map(|value| match value {
            Value::I64(code) => u32::try_from(*code)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| "invalid service character".into()),
            _ => Err("expected service character".into()),
        })
        .collect()
}

fn number(value: &Value) -> Result<i64> {
    match value {
        Value::I64(number) => Ok(*number),
        _ => Err("expected a service integer".into()),
    }
}

fn compile(source: &str) -> Result<ModuleSlot> {
    if source.len() > SOURCE_LIMIT {
        return Err("script exceeds 8192 bytes".into());
    }
    let mut depth = 0u32;
    for byte in source.bytes() {
        if byte == b'(' {
            depth += 1;
            if depth > 32 {
                return Err("script exceeds 32 levels of nesting".into());
            }
        } else if byte == b')' {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(ModuleSlot::new(Module::compile_compact(source)?, &[])?)
}

fn evaluate(script: &ModuleSlot, input: i64) -> Result<i64> {
    number(&script.run(Invocation {
        export: "propose",
        inputs: &[],
        arguments: &[Value::I64(input)],
        limits: Limits {
            instructions: 1024,
            cells: 256,
            frames: 8,
        },
    })?)
}

fn candidate(source: &str, tick: i64) -> Result<(i64, Option<Script>)> {
    if source.is_empty() {
        return Ok((-1, None));
    }
    let module = compile(source)?;
    let probe = evaluate(&module, tick)?;
    if probe == NO_PROBE {
        return Err("invalid script probe result".into());
    }
    Ok((
        probe,
        Some(Script {
            source: source.to_owned(),
            module,
        }),
    ))
}

type Invoke<'a> = dyn FnMut(&str, &[Value]) -> Result<Value> + 'a;

struct Event<'a> {
    kind: i64,
    slot: i64,
    epoch: i64,
    now: i64,
    args: &'a [i64],
    source: &'a str,
    probe: i64,
}

struct Reply {
    state: Value,
    status: i64,
    data: Arc<Vec<Value>>,
    source: Value,
}

impl Event<'_> {
    fn dispatch(&self, state: &Value, invoke: &mut Invoke<'_>) -> Result<Reply> {
        use Value::{I64, List, Record};
        let result = invoke(
            "app_service",
            &[
                state.clone(),
                I64(self.kind),
                I64(self.slot),
                I64(self.epoch),
                I64(self.now),
                integers(self.args.iter().copied()),
                integers(self.source.chars().map(|c| c as i64)),
                I64(self.probe),
            ],
        )?;
        let Record(result) = result else {
            return Err("expected a service reply record".into());
        };
        let [state, I64(status), List(data), source] = result.as_ref() else {
            return Err("expected state, service status, data and text".into());
        };
        Ok(Reply {
            state: state.clone(),
            status: *status,
            data: data.clone(),
            source: source.clone(),
        })
    }
}

impl Service {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let manifest = manifest(&path)?;
        let root = path.parent().ok_or("service manifest has no parent")?;
        let catalog = super::application::child(root, &manifest.catalog)?;
        let catalog = String::from_utf8(super::application::read(&catalog, 65536)?)?;
        let catalog = catalog
            .lines()
            .map(|line| line.split('\t').next().unwrap_or("").to_owned())
            .collect();
        Ok(Self {
            path,
            manifest,
            catalog,
            run: None,
            scripts: Default::default(),
        })
    }

    pub(super) fn pump(
        &mut self,
        state: &mut Value,
        now: i64,
        invoke: &mut Invoke<'_>,
    ) -> Result<()> {
        if let Some(run) = self.run.as_mut() {
            let (slot, epoch) = (run.slot, run.epoch);
            if let Some(status) = run.poll() {
                self.run = None;
                *state = Event {
                    kind: EFFECT_RESULT,
                    slot,
                    epoch,
                    now,
                    args: &[4, status],
                    source: "",
                    probe: NO_PROBE,
                }
                .dispatch(state, invoke)?
                .state;
            } else if let Ok(call) = run.requests.try_recv()
                && Instant::now() < call.deadline
            {
                let response = self
                    .call(state, &call, slot, epoch, now, invoke)
                    .unwrap_or_else(error);
                let _ = call.reply.send(response);
            }
        }
        let effect = Event {
            kind: EFFECT,
            slot: -1,
            epoch: -1,
            now,
            args: &[],
            source: "",
            probe: NO_PROBE,
        }
        .dispatch(state, invoke)?;
        *state = effect.state;
        if effect.status == 0 {
            return Ok(());
        }
        let [Value::I64(slot), Value::I64(epoch), Value::I64(input)] = effect.data.as_slice()
        else {
            return Err("expected service effect slot, epoch and input".into());
        };
        let result = match effect.status {
            1 => match Run::start(
                &self.path,
                &self.manifest,
                *slot,
                *epoch,
                &text(&effect.source)?,
            ) {
                Ok(run) => {
                    self.run = Some(run);
                    1
                }
                Err(error) => {
                    eprintln!("application service failed to start: {error}");
                    0
                }
            },
            2 => {
                self.run = None;
                1
            }
            3 => self
                .scripts
                .get(slot)
                .and_then(|script| evaluate(&script.module, *input).ok())
                .unwrap_or(NO_PROBE),
            5 => {
                let source = self.source(*slot);
                if !source.is_empty() {
                    macroquad::miniquad::window::clipboard_set(source);
                }
                i64::from(!source.is_empty())
            }
            6 => match self.paste(state, *slot, *epoch, now, invoke) {
                Ok(()) => 1,
                Err(_) => 0,
            },
            _ => return Err("unknown application service effect".into()),
        };
        *state = Event {
            kind: EFFECT_RESULT,
            slot: *slot,
            epoch: *epoch,
            now,
            args: &[effect.status, result],
            source: "",
            probe: NO_PROBE,
        }
        .dispatch(state, invoke)?
        .state;
        Ok(())
    }

    fn call(
        &mut self,
        state: &mut Value,
        call: &Call,
        slot: i64,
        epoch: i64,
        now: i64,
        invoke: &mut Invoke<'_>,
    ) -> Result<Json> {
        let (index, tool) = self
            .manifest
            .tools
            .iter()
            .enumerate()
            .find(|(_, tool)| tool.schema["name"].as_str() == Some(&call.tool))
            .ok_or("unknown application service tool")?;
        let args = call
            .arguments
            .as_object()
            .ok_or("expected tool arguments")?;
        let properties = &tool.schema["inputSchema"]["properties"];
        if args.keys().any(|key| properties.get(key).is_none()) {
            return Err("unknown tool argument".into());
        }
        if let Some(required) = tool.schema["inputSchema"]["required"].as_array()
            && required
                .iter()
                .any(|name| name.as_str().is_none_or(|name| !args.contains_key(name)))
        {
            return Err("missing required tool argument".into());
        }
        let mut values = tool
            .integers
            .iter()
            .map(|name| {
                args.get(name).map_or(Ok(-1), |value| {
                    value.as_i64().ok_or("expected integer argument")
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(name) = &tool.choice {
            let choice = args
                .get(name)
                .and_then(Json::as_str)
                .ok_or("expected named choice")?;
            values.push(
                self.catalog
                    .iter()
                    .position(|entry| entry == choice)
                    .ok_or("unknown order")? as i64,
            );
        }
        let source = tool
            .script
            .as_ref()
            .map(|name| {
                args.get(name)
                    .and_then(Json::as_str)
                    .ok_or("expected script source")
            })
            .transpose()?
            .unwrap_or("");
        let source = if source.trim().is_empty() { "" } else { source };
        if source.len() > SOURCE_LIMIT {
            return Err("script exceeds 8192 bytes".into());
        }
        let mut event = Event {
            kind: index as i64,
            slot,
            epoch,
            now,
            args: &values,
            source,
            probe: NO_PROBE,
        };
        let mut reply = event.dispatch(state, invoke)?;
        let mut script = None;
        if tool.script.is_some() && reply.status == 5 {
            let tick = number(reply.data.first().ok_or("expected script probe input")?)?;
            (event.probe, script) = candidate(source, tick)?;
            reply = event.dispatch(state, invoke)?;
        }
        if reply.status < 0 {
            let message = self
                .manifest
                .errors
                .get(reply.status.unsigned_abs() as usize - 1)
                .map(String::as_str)
                .unwrap_or("application rejected the request");
            return Ok(error(message));
        }
        let displayed = if tool.script.is_some() {
            source
        } else {
            self.source(slot)
        };
        let observed = self.observation(&reply, displayed)?;
        *state = reply.state;
        if tool.script.is_some() {
            self.install(slot, script);
        }
        Ok(
            json!({"content":[{"type":"text","text":observed.to_string()}],"structuredContent":observed}),
        )
    }

    fn install(&mut self, slot: i64, script: Option<Script>) {
        if let Some(script) = script {
            self.scripts.insert(slot, script);
        } else {
            self.scripts.remove(&slot);
        }
    }

    fn paste(
        &mut self,
        state: &mut Value,
        slot: i64,
        epoch: i64,
        now: i64,
        invoke: &mut Invoke<'_>,
    ) -> Result<()> {
        let source = macroquad::miniquad::window::clipboard_get().ok_or("clipboard is empty")?;
        if source.len() > SOURCE_LIMIT {
            return Err("script exceeds 8192 bytes".into());
        }
        let source = if source.trim().is_empty() {
            ""
        } else {
            &source
        };
        let mut event = Event {
            kind: LOCAL_SCRIPT,
            slot,
            epoch,
            now,
            args: &[],
            source,
            probe: NO_PROBE,
        };
        let reply = event.dispatch(state, invoke)?;
        if reply.status != 5 {
            return Err("application rejected clipboard script".into());
        }
        let tick = number(reply.data.first().ok_or("expected script probe input")?)?;
        let (probe, script) = candidate(source, tick)?;
        event.probe = probe;
        let reply = event.dispatch(state, invoke)?;
        if reply.status != 0 {
            return Err("application rejected clipboard script".into());
        }
        *state = reply.state;
        self.install(slot, script);
        Ok(())
    }

    fn source(&self, slot: i64) -> &str {
        self.scripts
            .get(&slot)
            .map_or("", |script| script.source.as_str())
    }

    pub(super) fn inspect(&self, state: &Value, now: i64, invoke: &mut Invoke<'_>) -> Result<Json> {
        let reply = Event {
            kind: INSPECT,
            slot: -1,
            epoch: -1,
            now,
            args: &[],
            source: "",
            probe: NO_PROBE,
        }
        .dispatch(state, invoke)?;
        let slot = number(
            reply
                .data
                .first()
                .ok_or("expected service inspection slot")?,
        )?;
        self.observation(&reply, self.source(slot))
    }

    fn observation(&self, reply: &Reply, source: &str) -> Result<Json> {
        let mut observed = serde_json::Map::new();
        let (fields, choices) = reply
            .data
            .split_at_checked(self.manifest.fields.len())
            .ok_or("incomplete service response")?;
        for (field, value) in self.manifest.fields.iter().zip(fields) {
            let value = number(value)?;
            observed.insert(
                field.clone(),
                if self.manifest.flags.contains(field) {
                    json!(value != 0)
                } else {
                    json!(value)
                },
            );
        }
        let choices = choices
            .iter()
            .map(|value| -> Result<&str> {
                self.catalog
                    .get(usize::try_from(number(value)?)?)
                    .map(String::as_str)
                    .ok_or_else(|| "unknown service response choice".into())
            })
            .collect::<Result<Vec<_>>>()?;
        observed.insert("choices".into(), json!(choices));
        observed.insert("source".into(), json!(source));
        Ok(Json::Object(observed))
    }
}

/// Stdio MCP adapter. The host capability is inherited only by the launched process.
pub fn serve(path: &Path) -> Result<()> {
    let manifest = manifest(path)?;
    let endpoint = Endpoint::Connect(env::var("FLUXR_SERVICE_ADDRESS")?.parse()?);
    let secret = secret_bytes(&env::var("FLUXR_SERVICE_SECRET")?)?;
    let tools: Vec<_> = manifest.tools.iter().map(|tool| &tool.schema).collect();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        if (&mut input)
            .take(PACKET as u64)
            .read_until(b'\n', &mut line)?
            == 0
        {
            return Ok(());
        }
        if line.len() == PACKET {
            return Err("MCP request exceeds packet limit".into());
        }
        let Ok(request) = serde_json::from_slice::<Json>(&line) else {
            writeln!(
                output,
                "{}",
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"invalid JSON"}})
            )?;
            output.flush()?;
            continue;
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let params = &request["params"];
        let result = match request["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion":params["protocolVersion"].as_str().unwrap_or("2024-11-05"),
                "capabilities":{"tools":{}},
                "serverInfo":{"name":manifest.namespace,"version":env!("CARGO_PKG_VERSION")}
            }),
            Some("ping") => json!({}),
            Some("tools/list") => json!({"tools":tools}),
            Some("tools/call") => (|| -> Result<Json> {
                let request = packet(&json!({"name":params["name"],
                    "arguments":params.get("arguments").cloned().unwrap_or(json!({}))}))?;
                let mut link = endpoint.open_with(&secret, DOMAIN, IPC_TIMEOUT, IPC_TIMEOUT)?;
                link.exchange(request)?;
                unpack(&link.exchange([0; PACKET])?)
            })()
            .unwrap_or_else(error),
            _ => {
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown method"}})
                )?;
                output.flush()?;
                continue;
            }
        };
        writeln!(
            output,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"result":result})
        )?;
        output.flush()?;
    }
}
