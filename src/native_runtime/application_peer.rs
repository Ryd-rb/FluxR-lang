//! Effect driver for an application-owned authenticated session.
use crate::{
    authenticated_tcp::{Endpoint, Link},
    structured::{Invocation, Limits, Module, Value, Workspace},
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    error::Error,
    fs::File,
    io::{self, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const LIMITS: Limits = Limits {
    instructions: 500_000,
    cells: 80_000,
    frames: 64,
};
const JOURNAL_LIMIT: usize = 262_145;

pub struct PeerRequest {
    listen: bool,
    address: SocketAddr,
}

impl PeerRequest {
    pub fn parse(role: &str, address: &str) -> Result<Self> {
        let listen = match role {
            "listen" => true,
            "connect" => false,
            _ => return Err("peer role must be listen or connect".into()),
        };
        Ok(Self {
            listen,
            address: address.parse()?,
        })
    }
}

#[derive(Default)]
struct Shared {
    snapshot: Option<(Value, Json)>,
    order: Option<i64>,
    error: Option<String>,
    accepting: bool,
}

pub(super) struct Peer {
    pub(super) slot: i64,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Peer {
    pub(super) fn start(
        module: Module,
        request: PeerRequest,
        key_env: &str,
        args: Value,
    ) -> Result<Self> {
        let secret = super::application_service::secret_bytes(
            &env::var(key_env)
                .map_err(|_| format!("set {key_env} to a shared 64-digit hex key"))?,
        )?;
        let endpoint = if request.listen {
            Endpoint::Listen(TcpListener::bind(request.address)?)
        } else {
            Endpoint::Connect(request.address)
        };
        let slot = endpoint.slot() as i64;
        let mut domain = Sha256::new();
        domain.update(b"fluxr/application-peer/v1");
        domain.update(module.canonical_source());
        domain.update(serde_json::to_vec(&args)?);
        let domain = domain.finalize().into();
        let mut program = Program {
            module,
            workspace: Workspace::default(),
        };
        let initial = program.invoke("app_peer_init", &[Value::I64(slot), args])?;
        if matches!(initial, Value::Variant(0, None)) {
            return Err("application rejected the peer launch arguments".into());
        }
        let Value::Variant(1, Some(initial)) = initial else {
            return Err("expected an optional application peer transition".into());
        };
        let trace = env::var_os("FLUXR_APP_PEER_TRACE")
            .map(PathBuf::from)
            .map(|path| {
                if !path.is_absolute() {
                    return Err(io::Error::other("peer trace path must be absolute"));
                }
                File::create_new(path)
            })
            .transpose()?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            endpoint,
            secret,
            domain,
            trace,
            shared: Arc::clone(&shared),
            stop: Arc::clone(&stop),
            link: None,
            journal: Vec::new(),
        };
        thread::Builder::new()
            .name("application-peer".into())
            .spawn(move || {
                if let Err(error) = worker.run(&mut program, initial.as_ref().clone()) {
                    let message = error.to_string();
                    eprintln!("application peer stopped: {message}");
                    worker.shared.lock().expect("peer snapshot").error = Some(message);
                }
            })?;
        Ok(Self { slot, shared, stop })
    }

    pub(super) fn snapshot(&self) -> (Option<(Value, Json)>, Option<String>) {
        let shared = self.shared.lock().expect("peer snapshot");
        (shared.snapshot.clone(), shared.error.clone())
    }

    pub(super) fn submit(&self, order: i64) -> bool {
        let mut shared = self.shared.lock().expect("peer snapshot");
        if !shared.accepting || shared.order.is_some() || shared.error.is_some() {
            return false;
        }
        shared.order = Some(order);
        true
    }
}

struct Program {
    module: Module,
    workspace: Workspace,
}

impl Program {
    fn invoke(&mut self, export: &str, arguments: &[Value]) -> Result<Value> {
        Ok(self
            .module
            .run_with(
                Invocation {
                    export,
                    arguments,
                    inputs: &[],
                    limits: LIMITS,
                },
                &mut self.workspace,
            )
            .map_err(|error| format!("{export}: {error}"))?)
    }
}

struct Worker {
    endpoint: Endpoint,
    secret: [u8; 32],
    domain: [u8; 32],
    trace: Option<File>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    link: Option<Link>,
    journal: Vec<Vec<i64>>,
}

pub(super) fn numbers(value: &Value) -> Result<Vec<i64>> {
    let Value::List(values) = value else {
        return Err("expected a peer word list".into());
    };
    values
        .iter()
        .map(|value| match value {
            Value::I64(number) => Ok(*number),
            _ => Err("expected a peer word".into()),
        })
        .collect()
}

fn words(bytes: &[u8]) -> impl Iterator<Item = i64> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .copied()
        .map(i64::from_be_bytes)
}

fn digest(value: &Value) -> Result<Vec<i64>> {
    let Value::Record(fields) = value else {
        return Err("expected a peer digest request".into());
    };
    let [world, prefix, suffix] = fields.as_ref() else {
        return Err("invalid peer digest request".into());
    };
    let world = Sha256::digest(serde_json::to_vec(world)?);
    let mut history = Sha256::new();
    for byte in numbers(prefix)? {
        history.update([u8::try_from(byte)?]);
    }
    for word in numbers(suffix)? {
        history.update(word.to_be_bytes());
    }
    Ok(words(&world).chain(words(&history.finalize())).collect())
}

fn failure(error: &io::Error) -> i64 {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput
    ) {
        2
    } else {
        1
    }
}

impl Worker {
    fn run(&mut self, program: &mut Program, mut transition: Value) -> Result<()> {
        while !self.stop.load(Ordering::Relaxed) {
            let Value::Record(fields) = transition else {
                return Err("expected a peer transition".into());
            };
            let [state, Value::Variant(effect, payload)] = fields.as_ref() else {
                return Err("expected peer state and effect".into());
            };
            let (status, words) =
                self.effect(*effect, payload.as_deref().unwrap_or(&Value::Unit))?;
            transition = program.invoke(
                "app_peer_event",
                &[
                    state.clone(),
                    Value::I64(status),
                    Value::List(Arc::new(words.into_iter().map(Value::I64).collect())),
                ],
            )?;
        }
        Ok(())
    }

    fn effect(&mut self, effect: usize, payload: &Value) -> Result<(i64, Vec<i64>)> {
        let mut reply = Vec::new();
        let mut status = 0;
        match (effect, payload) {
            (0, Value::I64(code)) => {
                return Err(format!("application ended the peer session ({code})").into());
            }
            (1, Value::I64(timeout)) => {
                let timeout = bounded_ms(*timeout, 10000)?;
                match self.endpoint.open_with(
                    &self.secret,
                    &self.domain,
                    Duration::from_millis(200),
                    timeout,
                ) {
                    Ok(link) => self.link = Some(link),
                    Err(error) => status = failure(&error),
                }
            }
            (2, payload) => {
                let request = numbers(payload)?;
                if !(1..=2048).contains(&request.len()) {
                    return Err("invalid peer exchange size".into());
                }
                let request = request
                    .into_iter()
                    .flat_map(i64::to_be_bytes)
                    .collect::<Vec<_>>();
                let mut response = vec![0; request.len()];
                let result = self
                    .link
                    .as_mut()
                    .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))
                    .and_then(|link| link.exchange_into(&request, &mut response));
                match result {
                    Ok(()) => reply = words(&response).collect(),
                    Err(error) => {
                        self.link = None;
                        status = failure(&error);
                    }
                }
            }
            (3, payload) => reply = digest(payload)?,
            (4, Value::Record(publication)) => {
                let [snapshot, report, Value::Bool(accepting)] = publication.as_ref() else {
                    return Err("invalid peer publication".into());
                };
                let report = json!(numbers(report)?);
                if let Some(trace) = &mut self.trace {
                    writeln!(trace, "{report}")?;
                    trace.flush()?;
                }
                let mut shared = self.shared.lock().expect("peer snapshot");
                shared.snapshot = Some((snapshot.clone(), report));
                shared.accepting = *accepting;
            }
            (5, Value::Record(wait)) => {
                let [Value::I64(delay), Value::Bool(collect)] = wait.as_ref() else {
                    return Err("invalid peer wait request".into());
                };
                let delay = bounded_ms(*delay, 10000)?;
                let deadline = Instant::now() + delay;
                while !self.stop.load(Ordering::Relaxed) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    thread::sleep(remaining.min(Duration::from_millis(20)));
                }
                if *collect
                    && let Some(order) = self.shared.lock().expect("peer snapshot").order.take()
                {
                    reply.push(order);
                }
            }
            (6, payload) => {
                let request = numbers(payload)?;
                let [start, count] = request.as_slice() else {
                    return Err("invalid journal read".into());
                };
                let start = usize::try_from(*start)?;
                let count = usize::try_from(*count)?;
                if count > 64 {
                    return Err("journal batch exceeds 64 rows".into());
                }
                if let Some(rows) = self.journal.get(start..start.saturating_add(count)) {
                    reply.extend(rows.iter().flatten().copied());
                } else {
                    status = 1;
                }
            }
            (7, payload) => {
                let entry = numbers(payload)?;
                if entry.len() > 32 || self.journal.len() >= JOURNAL_LIMIT {
                    return Err("application peer journal limit exceeded".into());
                }
                self.journal.push(entry);
            }
            (8, Value::Bool(clear)) => {
                self.link = None;
                if *clear {
                    let mut shared = self.shared.lock().expect("peer snapshot");
                    shared.order = None;
                    shared.accepting = false;
                }
            }
            _ => return Err("invalid application peer effect".into()),
        }
        Ok((status, reply))
    }
}

fn bounded_ms(delay: i64, maximum: u64) -> Result<Duration> {
    let delay = u64::try_from(delay)?;
    if delay > maximum {
        return Err("peer delay exceeds host limit".into());
    }
    Ok(Duration::from_millis(delay))
}
