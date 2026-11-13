//! The process runtime shared by compiled applications, source execution and
//! live injection. A revision is an immutable checked module; publication only
//! swaps an Arc. No registry lock is held while executing FluxR or native code.
pub use crate::structured::SourceDocument as Document;
use crate::structured::{
    self, HostServices, HostSignature, Invocation, Module, NativeKernel, OwnedPreparedAot,
    Workspace,
};
pub use crate::structured::{Binding, ExecutionRoute, Limits, Type, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const LANGUAGE_VERSION: u16 = 1;
pub const CONTROL_VERSION: u16 = 1;
pub const MAX_CONTROL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
}
impl RuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for RuntimeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RuntimeError {}
impl From<crate::Error> for RuntimeError {
    fn from(error: crate::Error) -> Self {
        Self::new("checked", error.to_string())
    }
}
type Result<T> = std::result::Result<T, RuntimeError>;

/// Bounds for one runtime. Native callbacks are trusted code and must set their
/// own I/O deadlines; VM fuel cannot preempt an arbitrary Rust function.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub max_modules: usize,
    pub max_names: usize,
    pub max_source_bytes: usize,
    pub max_active_bytes: usize,
    pub max_hosts: usize,
    pub max_instructions: u64,
    pub max_cells: usize,
    pub max_frames: usize,
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_modules: 256,
            max_names: 1024,
            max_source_bytes: 8 * 1024 * 1024,
            max_active_bytes: 32 * 1024 * 1024,
            max_hosts: 256,
            max_instructions: 50_000_000,
            max_cells: 1_000_000,
            max_frames: 256,
        }
    }
}
impl RuntimeConfig {
    fn check(&self) -> Result<()> {
        if self.max_modules == 0
            || self.max_modules > 4096
            || self.max_names < self.max_modules
            || self.max_names > 65536
            || self.max_source_bytes == 0
            || self.max_source_bytes > 8 * 1024 * 1024
            || self.max_active_bytes < self.max_source_bytes
            || self.max_active_bytes > 256 * 1024 * 1024
            || self.max_hosts == 0
            || self.max_hosts > 4096
            || self.max_instructions > 1_000_000_000
            || self.max_cells > 16_000_000
            || self.max_frames > 1024
        {
            return Err(RuntimeError::new(
                "configuration",
                "runtime bounds are invalid or exceed hard ceilings",
            ));
        }
        Ok(())
    }
    fn admit_limits(&self, limits: Limits) -> Result<()> {
        if limits.instructions > self.max_instructions
            || limits.cells > self.max_cells
            || limits.frames > self.max_frames
        {
            return Err(RuntimeError::new(
                "limits",
                "invocation exceeds runtime execution ceilings",
            ));
        }
        Ok(())
    }
}

type Callback = dyn Fn(&[Value]) -> Result<Value> + Send + Sync + 'static;
struct HostBinding {
    signature: HostSignature,
    callback: Arc<Callback>,
}
fn invoke_host(
    name: &str,
    binding: &HostBinding,
    arguments: &[Value],
) -> std::result::Result<Value, crate::Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (binding.callback)(arguments)
    })) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(structured::error(error.to_string())),
        Err(_) => Err(structured::error(format!(
            "host_panic: service {name} panicked"
        ))),
    }
}
struct PreparedServices {
    entries: BTreeMap<String, Arc<HostBinding>>,
}
impl HostServices for PreparedServices {
    fn call(&self, name: &str, arguments: &[Value]) -> std::result::Result<Value, crate::Error> {
        let binding = self
            .entries
            .get(name)
            .ok_or_else(|| structured::error("prepared host service is absent"))?;
        invoke_host(name, binding, arguments)
    }
}
#[derive(Default)]
struct Services {
    entries: RwLock<BTreeMap<String, Arc<HostBinding>>>,
}
impl HostServices for Services {
    fn call(&self, name: &str, arguments: &[Value]) -> std::result::Result<Value, crate::Error> {
        let binding = self
            .entries
            .read()
            .map_err(|_| structured::error("host registry lock poisoned"))?
            .get(name)
            .cloned()
            .ok_or_else(|| {
                structured::error(format!("host_missing: service {name} is not registered"))
            })?;
        // Drop the registry guard before user code, including recursive runtime
        // calls. Registrations are immutable, so the retained Arc stays valid.
        invoke_host(name, &binding, arguments)
    }
}

struct Revision {
    name: String,
    number: u64,
    module: Arc<Module>,
    source_sha256: String,
    source_bytes: usize,
    native_digest: Option<String>,
    requirements: BTreeMap<String, HostSignature>,
    host_exports: BTreeSet<String>,
    aot: RwLock<BTreeMap<String, Arc<OwnedPreparedAot>>>,
}
#[derive(Clone, Default)]
struct Registry {
    modules: BTreeMap<String, Arc<Revision>>,
    last_revision: BTreeMap<String, u64>,
    native_bindings: BTreeMap<String, String>,
    catalogs: BTreeMap<String, String>,
    active_bytes: usize,
}

struct PreparedModule {
    module: Arc<Module>,
    requirements: BTreeMap<String, HostSignature>,
    host_exports: BTreeSet<String>,
    source_sha256: String,
    source_bytes: usize,
    native_digest: String,
    has_native_items: bool,
}

pub struct Runtime {
    config: Arc<RuntimeConfig>,
    state: RwLock<Registry>,
    services: Arc<Services>,
    // Initialization compiles and executes this through the real VM, including
    // in native-only applications whose first injection arrives much later.
    _core: Arc<Module>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct ModuleSnapshot {
    revision: Arc<Revision>,
    services: Arc<Services>,
    config: Arc<RuntimeConfig>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleInfo {
    pub name: String,
    pub revision: u64,
    pub language_version: u16,
    pub source_sha256: String,
    pub source_bytes: usize,
    pub native_digest: Option<String>,
    pub inputs: Vec<Binding>,
    pub exports: BTreeMap<String, (Vec<Type>, Type)>,
    pub host_requirements: BTreeMap<String, HostSignature>,
    pub aot_exports: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallResult {
    pub revision: u64,
    pub value: Value,
    pub route: String,
}

/// A resolved execution tier for a retained revision. Keep the snapshot alive
/// while using this handle; new publications affect future lookups, never this
/// explicitly pinned call. Preparation performs all export and service lookups.
pub struct PreparedExport<'snapshot> {
    revision: u64,
    export: &'snapshot str,
    config: &'snapshot RuntimeConfig,
    tier: PreparedTier<'snapshot>,
}
enum PreparedTier<'a> {
    AheadOfTime(Arc<OwnedPreparedAot>),
    Checked(structured::PreparedCall<'a>),
    Host {
        call: structured::PreparedCall<'a>,
        services: PreparedServices,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedResult {
    pub revision: u64,
    pub value: Value,
    pub route: ExecutionRoute,
}
impl PreparedExport<'_> {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn call(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
    ) -> Result<PreparedResult> {
        self.call_with(inputs, arguments, limits, &mut Workspace::default())
    }
    pub fn call_with(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
    ) -> Result<PreparedResult> {
        self.config.admit_limits(limits)?;
        let (value, route) = match &self.tier {
            PreparedTier::AheadOfTime(aot) => aot.run_with_route(
                Invocation {
                    export: self.export,
                    inputs,
                    arguments,
                    limits,
                },
                workspace,
            ),
            PreparedTier::Checked(call) => {
                call.run_with_route(inputs, arguments, limits, workspace)
            }
            PreparedTier::Host { call, services } => (
                call.run_with_host(inputs, arguments, limits, workspace, services),
                ExecutionRoute::Portable,
            ),
        };
        Ok(PreparedResult {
            revision: self.revision,
            value: value.map_err(|error| RuntimeError::new("execution", error.to_string()))?,
            route,
        })
    }
}

impl ModuleSnapshot {
    pub fn revision(&self) -> u64 {
        self.revision.number
    }
    pub fn module(&self) -> &Arc<Module> {
        &self.revision.module
    }
    pub fn info(&self) -> Result<ModuleInfo> {
        revision_info(&self.revision)
    }
    pub fn prepare_export(&self, export: &str) -> Result<PreparedExport<'_>> {
        let (export, _) = self
            .revision
            .module
            .export_signatures()
            .get_key_value(export)
            .ok_or_else(|| RuntimeError::new("not_found", format!("unknown export {export}")))?;
        let aot = self
            .revision
            .aot
            .read()
            .map_err(|_| poisoned())?
            .get(export)
            .cloned();
        let tier = if let Some(aot) = aot {
            PreparedTier::AheadOfTime(aot)
        } else {
            let call = self.revision.module.prepare_call(export)?;
            if self.revision.host_exports.contains(export) {
                let requirements = self.revision.module.export_host_requirements(export)?;
                let registered = self.services.entries.read().map_err(|_| poisoned())?;
                let mut entries = BTreeMap::new();
                for (name, signature) in requirements {
                    let binding = registered.get(&name).ok_or_else(|| {
                        RuntimeError::new(
                            "host_missing",
                            format!("service {name} is not registered"),
                        )
                    })?;
                    if binding.signature != signature {
                        return Err(RuntimeError::new(
                            "host_signature",
                            format!("service {name} signature changed"),
                        ));
                    }
                    entries.insert(name, Arc::clone(binding));
                }
                PreparedTier::Host {
                    call,
                    services: PreparedServices { entries },
                }
            } else {
                PreparedTier::Checked(call)
            }
        };
        Ok(PreparedExport {
            revision: self.revision.number,
            export,
            config: &self.config,
            tier,
        })
    }
    pub fn call(
        &self,
        export: &str,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
    ) -> Result<CallResult> {
        self.call_with(export, inputs, arguments, limits, &mut Workspace::default())
    }
    pub fn call_with(
        &self,
        export: &str,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
        workspace: &mut Workspace,
    ) -> Result<CallResult> {
        self.config.admit_limits(limits)?;
        let invocation = Invocation {
            export,
            inputs,
            arguments,
            limits,
        };
        let aot = self
            .revision
            .aot
            .read()
            .map_err(|_| poisoned())?
            .get(export)
            .cloned();
        let (value, route) = if let Some(aot) = aot {
            aot.run_with_route(invocation, workspace)
        } else if self.revision.host_exports.contains(export) {
            (
                self.revision
                    .module
                    .run_with_host(invocation, workspace, &*self.services),
                ExecutionRoute::Portable,
            )
        } else {
            self.revision
                .module
                .prepare_call(export)?
                .run_with_route(inputs, arguments, limits, workspace)
        };
        Ok(CallResult {
            revision: self.revision.number,
            value: value.map_err(|error| RuntimeError::new("execution", error.to_string()))?,
            route: match route {
                ExecutionRoute::Portable => "vm",
                ExecutionRoute::AheadOfTime => "aot",
                ExecutionRoute::NativeScalar => "native_scalar",
                ExecutionRoute::TypedJit => "jit",
                ExecutionRoute::PrecompiledScan => "precompiled_scan",
            }
            .into(),
        })
    }
}

fn revision_info(revision: &Revision) -> Result<ModuleInfo> {
    Ok(ModuleInfo {
        name: revision.name.clone(),
        revision: revision.number,
        language_version: LANGUAGE_VERSION,
        source_sha256: revision.source_sha256.clone(),
        source_bytes: revision.source_bytes,
        native_digest: revision.native_digest.clone(),
        inputs: revision.module.input_bindings().to_vec(),
        exports: revision.module.export_signatures().clone(),
        host_requirements: revision.requirements.clone(),
        aot_exports: revision
            .aot
            .read()
            .map_err(|_| poisoned())?
            .keys()
            .cloned()
            .collect(),
    })
}
fn poisoned() -> RuntimeError {
    RuntimeError::new("internal", "runtime lock poisoned")
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/' | b':'))
}
fn host_type(ty: &Type, depth: usize) -> bool {
    depth < 32
        && match ty {
            Type::Unit | Type::Bool | Type::I64 | Type::U64 | Type::F32 => true,
            Type::List(element) => host_type(element, depth + 1),
            // A nominal name alone does not bind a native record layout. Services
            // use scalar/list values or explicit handles until schemas are bound.
            Type::Record(_) | Type::Enum(_) => false,
        }
}
fn admit_signature(signature: &HostSignature) -> Result<()> {
    if signature.params.len() > 256
        || !host_type(&signature.result, 0)
        || signature.params.iter().any(|ty| !host_type(ty, 0))
    {
        return Err(RuntimeError::new(
            "host_signature",
            "service ABI requires bounded scalar/list types; nominal Rust layouts are not implicit",
        ));
    }
    Ok(())
}

impl Runtime {
    pub fn new() -> Self {
        Self::with_config(RuntimeConfig::default()).expect("valid default runtime")
    }
    pub fn with_config(config: RuntimeConfig) -> Result<Self> {
        config.check()?;
        let core = Arc::new(Document::parse("fluxr 1\npub ready 0 unit ret unit")?.compile()?);
        core.run(Invocation {
            export: "ready",
            inputs: &[],
            arguments: &[],
            limits: Limits::default(),
        })?;
        Ok(Self {
            config: Arc::new(config),
            state: RwLock::new(Registry::default()),
            services: Arc::new(Services::default()),
            _core: core,
        })
    }

    /// Register a trusted native service once. An injected body can request an
    /// existing signature, but cannot register Rust code or change a binding.
    pub fn register_host<F>(
        &self,
        name: &str,
        params: Vec<Type>,
        result: Type,
        callback: F,
    ) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        if !valid_name(name)
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || name.as_bytes()[0].is_ascii_digit()
        {
            return Err(RuntimeError::new("name", "invalid host service name"));
        }
        let signature = HostSignature { params, result };
        admit_signature(&signature)?;
        // Lock order is always registry then services. Calls hold neither lock
        // across a callback. This also serializes catalog admission with binds.
        let state = self.state.read().map_err(|_| poisoned())?;
        for revision in state.modules.values() {
            if revision
                .requirements
                .get(name)
                .is_some_and(|required| *required != signature)
            {
                return Err(RuntimeError::new(
                    "host_signature",
                    format!("service {name} differs from a bundled requirement"),
                ));
            }
        }
        let mut services = self.services.entries.write().map_err(|_| poisoned())?;
        if services.contains_key(name) {
            return Err(RuntimeError::new(
                "host_exists",
                format!("service {name} is already registered"),
            ));
        }
        if services.len() >= self.config.max_hosts {
            return Err(RuntimeError::new(
                "capacity",
                "native service capacity reached",
            ));
        }
        services.insert(
            name.into(),
            Arc::new(HostBinding {
                signature,
                callback: Arc::new(callback),
            }),
        );
        Ok(())
    }

    pub fn install(
        &self,
        name: &str,
        expected_revision: Option<u64>,
        source: &str,
    ) -> Result<ModuleInfo> {
        self.install_inner(name, expected_revision, source, None)
    }

    /// A build-provided native catalog is a trusted admission operation. It is
    /// intentionally absent from the remote protocol. Its digest binds exact
    /// native source bytes, while checked bodies still use the ordinary loader.
    pub fn install_embedded(&self, source: EmbeddedSource) -> Result<ModuleInfo> {
        self.install_inner(source.name, None, source.source, Some(source.native_digest))
    }

    /// Publish a complete build catalog atomically. Parse and compilation occur
    /// outside the publication lock; conflicts, bounds and service grants are
    /// checked against a staged registry before the single final swap.
    pub fn install_embedded_sources(&self, sources: &[EmbeddedSource]) -> Result<Vec<ModuleInfo>> {
        if sources.len() > self.config.max_modules {
            return Err(RuntimeError::new(
                "capacity",
                "embedded catalog exceeds module capacity",
            ));
        }
        let mut names = BTreeSet::new();
        let mut total = 0usize;
        let mut prepared = Vec::new();
        for source in sources {
            if !names.insert(source.name) {
                return Err(RuntimeError::new(
                    "conflict",
                    "duplicate embedded module name",
                ));
            }
            total = total.saturating_add(source.source.len());
            if total > self.config.max_active_bytes {
                return Err(RuntimeError::new(
                    "capacity",
                    "embedded catalog exceeds active source capacity",
                ));
            }
            prepared.push((
                source.name,
                self.prepare(source.name, source.source, Some(source.native_digest))?,
            ));
        }
        let mut state = self.state.write().map_err(|_| poisoned())?;
        let mut staged = state.clone();
        let mut infos = Vec::new();
        for (name, candidate) in prepared {
            infos.push(self.publish(&mut staged, name, None, candidate, true)?);
        }
        *state = staged;
        Ok(infos)
    }

    fn install_inner(
        &self,
        name: &str,
        expected_revision: Option<u64>,
        source: &str,
        build_digest: Option<&str>,
    ) -> Result<ModuleInfo> {
        let candidate = self.prepare(name, source, build_digest)?;
        let mut state = self.state.write().map_err(|_| poisoned())?;
        self.publish(
            &mut state,
            name,
            expected_revision,
            candidate,
            build_digest.is_some(),
        )
    }

    fn prepare(
        &self,
        name: &str,
        source: &str,
        build_digest: Option<&str>,
    ) -> Result<PreparedModule> {
        if !valid_name(name) {
            return Err(RuntimeError::new("name", "invalid module name"));
        }
        if source.len() > self.config.max_source_bytes {
            return Err(RuntimeError::new(
                "source_limit",
                "source exceeds runtime limit",
            ));
        }
        let document = Document::parse(source)?;
        if let Some(digest) = build_digest
            && digest != document.native_digest
        {
            return Err(RuntimeError::new(
                "native_identity",
                "build catalog native digest differs from source",
            ));
        }
        let module = Arc::new(document.compile()?);
        let requirements = module.host_requirements()?;
        for signature in requirements.values() {
            admit_signature(signature)?;
        }
        let host_exports = module
            .export_signatures()
            .keys()
            .filter_map(|name| module.export_uses_host(name).then(|| name.clone()))
            .collect();
        let source_sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        Ok(PreparedModule {
            module,
            requirements,
            host_exports,
            source_sha256,
            source_bytes: source.len(),
            has_native_items: document.has_native_items(),
            native_digest: document.native_digest,
        })
    }

    fn publish(
        &self,
        state: &mut Registry,
        name: &str,
        expected_revision: Option<u64>,
        candidate: PreparedModule,
        trusted_catalog: bool,
    ) -> Result<ModuleInfo> {
        let PreparedModule {
            module,
            requirements,
            host_exports,
            source_sha256,
            source_bytes,
            native_digest,
            has_native_items,
        } = candidate;
        if trusted_catalog
            && state.catalogs.get(name) == Some(&source_sha256)
            && let Some(revision) = state.modules.get(name)
        {
            return revision_info(revision);
        }
        let prior = state.modules.get(name);
        match (expected_revision, prior) {
            (None, Some(_)) => {
                return Err(RuntimeError::new(
                    "conflict",
                    "module already exists; replacement requires its revision",
                ));
            }
            (Some(expected), Some(prior)) if expected == prior.number => {
                if !prior.module.same_interface(&module) {
                    return Err(RuntimeError::new(
                        "interface",
                        "replacement changes the checked interface",
                    ));
                }
            }
            (Some(_), _) => {
                return Err(RuntimeError::new(
                    "conflict",
                    "expected module revision is not active",
                ));
            }
            (None, None) => {}
        }
        if let Some(bound) = state.native_bindings.get(name) {
            if *bound != native_digest {
                return Err(RuntimeError::new(
                    "native_identity",
                    "native extensions differ from the compiled binding; rebuild required",
                ));
            }
        } else if has_native_items && !trusted_catalog {
            return Err(RuntimeError::new(
                "native_unbound",
                "native extensions have no build-provided binding; compile the program first",
            ));
        }
        let services = self.services.entries.read().map_err(|_| poisoned())?;
        for (name, signature) in &requirements {
            if state.modules.values().any(|revision| {
                revision
                    .requirements
                    .get(name)
                    .is_some_and(|required| required != signature)
            }) {
                return Err(RuntimeError::new(
                    "host_signature",
                    format!("service {name} differs from an existing module requirement"),
                ));
            }
            match services.get(name) {
                Some(binding) if binding.signature == *signature => {}
                Some(_) => {
                    return Err(RuntimeError::new(
                        "host_signature",
                        format!("service {name} signature differs from native binding"),
                    ));
                }
                None if trusted_catalog => {}
                None => {
                    return Err(RuntimeError::new(
                        "host_missing",
                        format!("service {name} is not registered"),
                    ));
                }
            }
        }
        drop(services);
        if prior.is_none() && state.modules.len() >= self.config.max_modules {
            return Err(RuntimeError::new(
                "capacity",
                "active module capacity reached",
            ));
        }
        if !state.last_revision.contains_key(name)
            && state.last_revision.len() >= self.config.max_names
        {
            return Err(RuntimeError::new(
                "capacity",
                "module name history capacity reached",
            ));
        }
        let active_bytes = state.active_bytes - prior.map_or(0, |r| r.source_bytes) + source_bytes;
        if active_bytes > self.config.max_active_bytes {
            return Err(RuntimeError::new(
                "capacity",
                "active source byte capacity reached",
            ));
        }
        let number = state
            .last_revision
            .get(name)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new("capacity", "module revision counter exhausted"))?;
        let revision = Arc::new(Revision {
            name: name.into(),
            number,
            module,
            source_sha256: source_sha256.clone(),
            source_bytes,
            native_digest: has_native_items.then(|| native_digest.clone()),
            requirements,
            host_exports,
            aot: RwLock::new(BTreeMap::new()),
        });
        let info = revision_info(&revision)?;
        if trusted_catalog {
            state.catalogs.insert(name.into(), source_sha256);
            if has_native_items {
                state.native_bindings.insert(name.into(), native_digest);
            }
        }
        state.active_bytes = active_bytes;
        state.last_revision.insert(name.into(), number);
        state.modules.insert(name.into(), revision);
        Ok(info)
    }

    pub fn snapshot(&self, name: &str) -> Result<ModuleSnapshot> {
        let revision = self
            .state
            .read()
            .map_err(|_| poisoned())?
            .modules
            .get(name)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("not_found", format!("module {name} is not installed"))
            })?;
        Ok(ModuleSnapshot {
            revision,
            services: Arc::clone(&self.services),
            config: Arc::clone(&self.config),
        })
    }
    pub fn call(
        &self,
        name: &str,
        export: &str,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
    ) -> Result<CallResult> {
        self.snapshot(name)?.call(export, inputs, arguments, limits)
    }
    pub fn inspect(&self, name: &str) -> Result<ModuleInfo> {
        self.snapshot(name)?.info()
    }
    pub fn list(&self) -> Result<Vec<ModuleInfo>> {
        self.state
            .read()
            .map_err(|_| poisoned())?
            .modules
            .values()
            .map(|r| revision_info(r))
            .collect()
    }
    pub fn remove(&self, name: &str, expected_revision: u64) -> Result<ModuleInfo> {
        let mut state = self.state.write().map_err(|_| poisoned())?;
        let prior = state
            .modules
            .get(name)
            .ok_or_else(|| RuntimeError::new("not_found", "module is not installed"))?;
        if prior.number != expected_revision {
            return Err(RuntimeError::new(
                "conflict",
                "expected module revision is not active",
            ));
        }
        let info = revision_info(prior)?;
        state.active_bytes -= prior.source_bytes;
        state.modules.remove(name);
        // Retain the counter so remove/recreate cannot produce an ABA revision.
        Ok(info)
    }
    pub fn bind_aot(
        &self,
        name: &str,
        expected_revision: u64,
        export: &str,
        identity: [u8; 32],
        kernel: NativeKernel,
    ) -> Result<()> {
        let snapshot = self.snapshot(name)?;
        if snapshot.revision() != expected_revision {
            return Err(RuntimeError::new("conflict", "AOT revision is not active"));
        }
        let binding = Arc::new(
            snapshot
                .module()
                .prepare_owned_aot(export, identity, kernel)?,
        );
        let state = self.state.read().map_err(|_| poisoned())?;
        let active = state
            .modules
            .get(name)
            .ok_or_else(|| RuntimeError::new("conflict", "AOT module was removed"))?;
        if !Arc::ptr_eq(active, &snapshot.revision) {
            return Err(RuntimeError::new(
                "conflict",
                "AOT module changed during binding",
            ));
        }
        let mut bindings = active.aot.write().map_err(|_| poisoned())?;
        if bindings.contains_key(export) {
            return Err(RuntimeError::new(
                "aot_exists",
                "export already has an AOT binding",
            ));
        }
        bindings.insert(export.into(), binding);
        Ok(())
    }

    pub fn execute(&self, command: Command) -> ControlResponse {
        let result = match command {
            Command::Install {
                name,
                expected_revision,
                source,
            } => self
                .install(&name, expected_revision, &source)
                .and_then(json),
            Command::Call {
                name,
                export,
                inputs,
                arguments,
                limits,
            } => self
                .call(&name, &export, &inputs, &arguments, limits)
                .and_then(json),
            Command::Inspect { name } => self.inspect(&name).and_then(json),
            Command::List => self.list().and_then(json),
            Command::Remove {
                name,
                expected_revision,
            } => self.remove(&name, expected_revision).and_then(json),
        };
        ControlResponse::from_result(result)
    }
}
fn json(value: impl Serialize) -> Result<serde_json::Value> {
    // Bound the encoded tree before allocating serde_json::Value's per-node
    // containers. This applies equally to stdio, TCP, and in-process commands.
    let bytes = bounded_json(&value, MAX_CONTROL_BYTES - 512)?;
    serde_json::from_slice(&bytes).map_err(|e| RuntimeError::new("encoding", e.to_string()))
}
fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.len().saturating_add(bytes.len()) > self.limit {
                return Err(std::io::Error::other("control result exceeds byte limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value)
        .map_err(|e| RuntimeError::new("output_limit", e.to_string()))?;
    Ok(output.bytes)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Install {
        name: String,
        expected_revision: Option<u64>,
        source: String,
    },
    Call {
        name: String,
        export: String,
        #[serde(default)]
        inputs: Vec<Value>,
        #[serde(default)]
        arguments: Vec<Value>,
        #[serde(default)]
        limits: Limits,
    },
    Inspect {
        name: String,
    },
    List,
    Remove {
        name: String,
        expected_revision: u64,
    },
}
impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // Serde's internally tagged unit variant ignores extra fields even
        // with deny_unknown_fields. An empty struct variant enforces the same
        // strict map contract as every other operation, without changing the
        // public Rust Command::List constructor or JSON representation.
        #[derive(Deserialize)]
        #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
        enum WireCommand {
            Install {
                name: String,
                expected_revision: Option<u64>,
                source: String,
            },
            Call {
                name: String,
                export: String,
                #[serde(default)]
                inputs: Vec<Value>,
                #[serde(default)]
                arguments: Vec<Value>,
                #[serde(default)]
                limits: Limits,
            },
            Inspect {
                name: String,
            },
            List {},
            Remove {
                name: String,
                expected_revision: u64,
            },
        }
        Ok(match WireCommand::deserialize(deserializer)? {
            WireCommand::Install {
                name,
                expected_revision,
                source,
            } => Self::Install {
                name,
                expected_revision,
                source,
            },
            WireCommand::Call {
                name,
                export,
                inputs,
                arguments,
                limits,
            } => Self::Call {
                name,
                export,
                inputs,
                arguments,
                limits,
            },
            WireCommand::Inspect { name } => Self::Inspect { name },
            WireCommand::List {} => Self::List,
            WireCommand::Remove {
                name,
                expected_revision,
            } => Self::Remove {
                name,
                expected_revision,
            },
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub version: u16,
    pub token: String,
    pub command: Command,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlResponse {
    pub version: u16,
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<RuntimeError>,
}
impl ControlResponse {
    pub fn from_result(result: Result<serde_json::Value>) -> Self {
        match result {
            Ok(result) => Self {
                version: CONTROL_VERSION,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                version: CONTROL_VERSION,
                ok: false,
                result: None,
                error: Some(error),
            },
        }
    }
}
pub fn handle_control_request(
    runtime: &Runtime,
    request: ControlRequest,
    expected_token: &str,
) -> ControlResponse {
    if request.version != CONTROL_VERSION {
        return ControlResponse::from_result(Err(RuntimeError::new(
            "version",
            "unsupported control protocol version",
        )));
    }
    if !token_equal(request.token.as_bytes(), expected_token.as_bytes()) {
        return ControlResponse::from_result(Err(RuntimeError::new(
            "authentication",
            "control token differs",
        )));
    }
    runtime.execute(request.command)
}
fn token_equal(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for (i, &byte) in b.iter().enumerate() {
        difference |= usize::from(byte ^ a.get(i).copied().unwrap_or(0));
    }
    difference == 0
}

#[derive(Clone, Copy, Debug)]
pub enum ImportKind {
    Agent,
    Compact,
    Fsm,
    Wordcode,
}
pub fn import_source(kind: ImportKind, bytes: &[u8]) -> Result<String> {
    let text =
        || std::str::from_utf8(bytes).map_err(|e| RuntimeError::new("encoding", e.to_string()));
    let module = match kind {
        ImportKind::Agent => Module::compile_agent(text()?)?,
        ImportKind::Compact => Module::compile_compact(text()?)?,
        ImportKind::Fsm => Module::decode(bytes)?,
        ImportKind::Wordcode => Module::compile_wordcode(
            text()?,
            &structured::wordcode::Dictionary::core(),
            Default::default(),
        )?,
    };
    Ok(module.unified_text()?)
}

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedSource {
    pub name: &'static str,
    pub source: &'static str,
    pub native_digest: &'static str,
}
static GLOBAL: OnceLock<Arc<Runtime>> = OnceLock::new();
static CONTROL: OnceLock<Mutex<Weak<ControlService>>> = OnceLock::new();
pub fn global() -> &'static Arc<Runtime> {
    GLOBAL.get_or_init(|| Arc::new(Runtime::new()))
}

pub struct RuntimeGuard {
    runtime: Arc<Runtime>,
    control: Option<Arc<ControlService>>,
}
impl RuntimeGuard {
    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }
    pub fn control_address(&self) -> Option<SocketAddr> {
        self.control.as_ref().map(|x| x.address)
    }
}
pub fn start_embedded() -> Result<RuntimeGuard> {
    start_embedded_with_sources(&[])
}
pub fn start_embedded_with_sources(sources: &[EmbeddedSource]) -> Result<RuntimeGuard> {
    let runtime = Arc::clone(global());
    runtime.install_embedded_sources(sources)?;
    let mut control = CONTROL
        .get_or_init(|| Mutex::new(Weak::new()))
        .lock()
        .map_err(|_| poisoned())?;
    let control = if let Some(existing) = control.upgrade() {
        Some(existing)
    } else if let Some(address) = std::env::var_os("FLUXR_CONTROL_ADDR") {
        let address = address
            .to_str()
            .ok_or_else(|| RuntimeError::new("configuration", "control address is not UTF-8"))?
            .parse::<SocketAddr>()
            .map_err(|e| RuntimeError::new("configuration", e.to_string()))?;
        let token = std::env::var("FLUXR_CONTROL_TOKEN").map_err(|_| {
            RuntimeError::new(
                "configuration",
                "control endpoint requires FLUXR_CONTROL_TOKEN",
            )
        })?;
        let service = Arc::new(ControlService::start(
            Arc::clone(&runtime),
            address,
            token,
            std::env::var_os("FLUXR_CONTROL_FILE").map(PathBuf::from),
        )?);
        *control = Arc::downgrade(&service);
        Some(service)
    } else {
        None
    };
    Ok(RuntimeGuard { runtime, control })
}
pub fn run_embedded_entry(module_name: &str, export: &str) -> Result<()> {
    let call = global().call(module_name, export, &[], &[], Limits::default())?;
    if call.value != Value::Unit {
        let stdout = std::io::stdout();
        let mut output = stdout.lock();
        serde_json::to_writer(&mut output, &call.value)
            .map_err(|e| RuntimeError::new("output", e.to_string()))?;
        output
            .write_all(b"\n")
            .map_err(|e| RuntimeError::new("output", e.to_string()))?;
    }
    Ok(())
}

/// Bounded authenticated loopback NDJSON. A connection carries one request;
/// framing and authentication run before the operation dispatcher. The caller
/// owns this service's lifetime, independently of the process-global runtime.
pub struct ControlService {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    receipt: Option<(PathBuf, std::fs::Metadata)>,
}
impl ControlService {
    pub fn start(
        runtime: Arc<Runtime>,
        address: SocketAddr,
        token: String,
        receipt_path: Option<PathBuf>,
    ) -> Result<Self> {
        if !address.ip().is_loopback() {
            return Err(RuntimeError::new(
                "configuration",
                "control endpoint must bind a loopback IP",
            ));
        }
        if token.len() < 32 || token.len() > 1024 {
            return Err(RuntimeError::new(
                "configuration",
                "control token must contain 32 through 1024 bytes",
            ));
        }
        let listener = TcpListener::bind(address).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let receipt = if let Some(path) = receipt_path {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path).map_err(io_error)?;
            let data = serde_json::to_vec(&serde_json::json!({"version": CONTROL_VERSION,
                "language_version": LANGUAGE_VERSION, "address": address.to_string(), "pid": std::process::id()}))
                .map_err(|e| RuntimeError::new("encoding", e.to_string()))?;
            if let Err(error) = file.write_all(&data).and_then(|_| file.sync_all()) {
                let _ = std::fs::remove_file(&path);
                return Err(io_error(error));
            }
            Some((path, file.metadata().map_err(io_error)?))
        } else {
            None
        };
        let stopping = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let worker = thread::Builder::new()
            .name("fluxr-control".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) if peer.ip().is_loopback() => {
                            if active
                                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                                    (n < 8).then_some(n + 1)
                                })
                                .is_err()
                            {
                                continue;
                            }
                            let runtime = Arc::clone(&runtime);
                            let token = token.clone();
                            let active = Arc::clone(&active);
                            let rollback = Arc::clone(&active);
                            if thread::Builder::new()
                                .name("fluxr-request".into())
                                .spawn(move || {
                                    struct Active(Arc<AtomicUsize>);
                                    impl Drop for Active {
                                        fn drop(&mut self) {
                                            self.0.fetch_sub(1, Ordering::AcqRel);
                                        }
                                    }
                                    let _active = Active(active);
                                    let _ = handle_connection(runtime, stream, &token);
                                })
                                .is_err()
                            {
                                rollback.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(io_error)?;
        Ok(Self {
            address,
            stop,
            worker: Some(worker),
            receipt,
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}
impl Drop for ControlService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // In-flight requests own revision snapshots and may finish after this
        // listener closes. Native services supply their own cancellation policy.
        if let Some((path, original)) = self.receipt.take()
            && let Ok(current) = std::fs::symlink_metadata(&path)
        {
            #[cfg(unix)]
            let same = {
                use std::os::unix::fs::MetadataExt;
                current.dev() == original.dev() && current.ino() == original.ino()
            };
            #[cfg(not(unix))]
            let same = current.is_file()
                && current.len() == original.len()
                && current.modified().ok() == original.modified().ok();
            if same {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
fn io_error(error: std::io::Error) -> RuntimeError {
    RuntimeError::new("io", error.to_string())
}
fn handle_connection(runtime: Arc<Runtime>, mut stream: TcpStream, token: &str) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(io_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(io_error)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = Vec::new();
    let response = loop {
        if Instant::now() >= deadline {
            break ControlResponse::from_result(Err(RuntimeError::new(
                "timeout",
                "control request framing deadline exceeded",
            )));
        }
        let mut buffer = [0u8; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => {
                break ControlResponse::from_result(Err(RuntimeError::new(
                    "framing",
                    "control request requires a newline",
                )));
            }
            Ok(count) => {
                let end = buffer[..count].iter().position(|&b| b == b'\n');
                let take = end.map_or(count, |i| i + 1);
                if line.len() + take > MAX_CONTROL_BYTES {
                    break ControlResponse::from_result(Err(RuntimeError::new(
                        "input_limit",
                        "control request exceeds byte limit",
                    )));
                }
                line.extend_from_slice(&buffer[..take]);
                if end.is_some() {
                    break match serde_json::from_slice::<ControlRequest>(&line) {
                        Ok(request) => handle_control_request(&runtime, request, token),
                        Err(error) => ControlResponse::from_result(Err(RuntimeError::new(
                            "invalid_request",
                            error.to_string(),
                        ))),
                    };
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(io_error(error)),
        }
    };
    let mut payload = match bounded_json(&response, MAX_CONTROL_BYTES - 1) {
        Ok(payload) => payload,
        Err(error) => bounded_json(
            &ControlResponse::from_result(Err(error)),
            MAX_CONTROL_BYTES - 1,
        )?,
    };
    payload.push(b'\n');
    stream.write_all(&payload).map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_writer_stops_before_the_overflowing_fragment() {
        let small = serde_json::json!({"text": "abc"});
        let length = serde_json::to_vec(&small).unwrap().len();
        assert_eq!(bounded_json(&small, length).unwrap().len(), length);
        assert_eq!(
            bounded_json(&small, length - 1).unwrap_err().code,
            "output_limit"
        );
        // The serializer receives a borrowed long string: no complete encoded
        // copy is allocated merely to discover it exceeds the limit.
        let long = "x".repeat(MAX_CONTROL_BYTES + 1);
        assert_eq!(
            bounded_json(&long, MAX_CONTROL_BYTES).unwrap_err().code,
            "output_limit"
        );
    }

    #[test]
    fn every_protocol_map_rejects_unknown_or_duplicate_fields() {
        for command in [
            r#"{"op":"list","extra":0}"#,
            r#"{"op":"list","op":"list"}"#,
            r#"{"op":"inspect","name":"x","extra":0}"#,
            r#"{"op":"remove","name":"x","expected_revision":1,"extra":0}"#,
            r#"{"op":"install","name":"x","expected_revision":null,"source":"fluxr 1\n","extra":0}"#,
            r#"{"op":"call","name":"x","export":"x","limits":{"instructions":1,"cells":1,"frames":1,"extra":0}}"#,
        ] {
            assert!(
                serde_json::from_str::<Command>(command).is_err(),
                "{command}"
            );
        }
        for request in [
            r#"{"version":1,"version":1,"token":"","command":{"op":"list"}}"#,
            r#"{"version":1,"token":"","command":{"op":"list"},"extra":0}"#,
        ] {
            assert!(
                serde_json::from_str::<ControlRequest>(request).is_err(),
                "{request}"
            );
        }
        assert!(matches!(
            serde_json::from_str::<Command>(r#"{"op":"list"}"#).unwrap(),
            Command::List
        ));
    }
}
