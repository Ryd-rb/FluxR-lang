//! Bounded tests for checked FluxR source, independent of compiler syntax.
//!
//! A `fluxr-tests-v1` JSON catalog contains named cases, source paths relative
//! to the catalog, arguments/inputs, and exactly one expected value, runtime
//! error substring, or compilation-error substring. Native Cargo/libtest
//! projects continue to use the existing builder. This runner neither executes
//! Rust source spans nor supplies native services, and is not a hostile-code or
//! concurrently-mutated-filesystem sandbox.
use crate::runtime::Document;
use crate::structured::{Invocation, Limits, Module, PreparedCall, Type, Value, Workspace};
use serde::{Deserialize, Serialize, de};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const MAX_CATALOG: usize = 8 * 1024 * 1024;
const MAX_SOURCE: usize = 8 * 1024 * 1024;
const MAX_SOURCE_TOTAL: usize = 32 * 1024 * 1024;
const MAX_TESTS: usize = 4096;
const MAX_CALLS: u64 = 65_536;
const MAX_FUEL: u64 = 1_000_000_000;
const MAX_SHRINK: usize = 128;
const MAX_LIST_LENGTH: usize = 4096;
const MAX_GENERATED_CELLS: usize = 262_144;
// The existing scalar-only ceiling: 64 argument cells per reserved call.
const MAX_GENERATED_CELL_WORK: u64 = MAX_CALLS * 64;

#[derive(Debug)]
pub struct HarnessError(String);
impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Error for HarnessError {}
fn error(message: impl Into<String>) -> HarnessError {
    HarnessError(message.into())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub format: String,
    pub tests: Vec<TestCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub export: String,
    #[serde(default)]
    pub arguments: Vec<Value>,
    #[serde(default)]
    pub inputs: Vec<Value>,
    pub expect: Expectation,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub ignore: Option<String>,
    #[serde(default)]
    pub property: Option<Property>,
}

/// Error matching is a nonempty substring of an error from the named phase.
/// File errors, unknown exports and unsupported native spans never satisfy it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Expectation {
    Value { value: Value },
    Error { error: String },
    CompileError { compile_error: String },
}

/// Reproducible Boolean properties use SplitMix64 with scalar or flat-list
/// argument domains. List lengths and elements have inclusive declared bounds.
/// Fixed arguments are forbidden; fixed host snapshot inputs remain available.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub seed: u64,
    pub cases: usize,
    pub generators: Vec<Generator>,
    #[serde(default)]
    pub shrink: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Generator {
    Bool {},
    I64 {
        min: i64,
        max: i64,
    },
    U64 {
        min: u64,
        max: u64,
    },
    BoolList {
        min_len: usize,
        max_len: usize,
    },
    I64List {
        min_len: usize,
        max_len: usize,
        min: i64,
        max: i64,
    },
    U64List {
        min_len: usize,
        max_len: usize,
        min: u64,
        max: u64,
    },
}

fn max_generated_cells(generator: &Generator) -> usize {
    // Lengths are admitted before this helper is used. Count the outer list
    // even when it is empty; these host allocations are separate from VM cells.
    match *generator {
        Generator::BoolList { max_len, .. }
        | Generator::I64List { max_len, .. }
        | Generator::U64List { max_len, .. } => 1 + max_len,
        _ => 1,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Portable,
    #[default]
    Prepared,
    Compare,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Serialize)]
pub struct Options {
    pub filter: Option<String>,
    pub exact: bool,
    pub skip: Vec<String>,
    pub ignored_only: bool,
    pub include_ignored: bool,
    pub list: bool,
    pub fail_fast: bool,
    pub allow_empty: bool,
    pub repeat: usize,
    pub test_threads: usize,
    pub mode: Mode,
    pub output: Output,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            filter: None,
            exact: false,
            skip: vec![],
            ignored_only: false,
            include_ignored: false,
            list: false,
            fail_fast: false,
            allow_empty: false,
            repeat: 1,
            test_threads: 1,
            mode: Mode::Prepared,
            output: Output::Text,
        }
    }
}

pub struct Suite {
    catalog: Catalog,
    identity: CatalogIdentity,
    sources: BTreeMap<String, String>,
}
impl Suite {
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}

/// Identity of the exact catalog bytes, including expectations, seeds and
/// resource settings; not an authentication or independent execution proof.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogIdentity {
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceIdentity {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub format: &'static str,
    pub catalog: CatalogIdentity,
    pub configuration: Options,
    pub success: bool,
    pub listed: bool,
    pub selected: usize,
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub filtered: usize,
    pub not_run: usize,
    pub sources: Vec<SourceIdentity>,
    pub tests: Vec<TestResult>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Ignored,
    Listed,
    NotRun,
}
#[derive(Clone, Debug, Serialize)]
pub struct TestResult {
    pub name: String,
    pub status: Status,
    /// Runtime invocations including shrink attempts, excluding extra portable
    /// comparisons. Compilation expectations have zero runtime invocations.
    pub executions: usize,
    pub routes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<Replay>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Replay {
    pub seed: u64,
    pub case: usize,
    pub repetition: usize,
    pub arguments: Vec<Value>,
    pub shrunk_arguments: Vec<Value>,
    pub shrink_evaluations: usize,
}

// Deserialize maps before typed conversion so duplicate keys cannot disappear
// in untagged-enum buffering or nested JSON values.
struct UniqueJson(serde_json::Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueJson(n.into()))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = a.next_element::<UniqueJson>()? {
                    values.push(v.0);
                }
                Ok(UniqueJson(values.into()))
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = a.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
                    }
                    values.insert(key, a.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(values.into()))
            }
        }
        d.deserialize_any(Visitor)
    }
}

fn read_file(path: &Path, max: usize) -> Result<Vec<u8>, HarnessError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| error(format!("{}: {e}", path.display())))?;
    if !metadata.file_type().is_file() {
        return Err(error(format!(
            "{} must be a regular, non-symlink file",
            path.display()
        )));
    }
    if metadata.len() > max as u64 {
        return Err(error("file exceeds its byte limit"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(max as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| error(format!("{}: {e}", path.display())))?;
    if bytes.len() > max {
        return Err(error("file exceeds its byte limit"));
    }
    Ok(bytes)
}

fn text_valid(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}
fn source_path(root: &Path, text: &str) -> Result<PathBuf, HarnessError> {
    if !text_valid(text, 4096)
        || text.contains(['\\', ':'])
        || Path::new(text).is_absolute()
        || text
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.ends_with([' ', '.']))
    {
        return Err(error("source must be a safe catalog-relative path"));
    }
    let mut result = root.to_owned();
    for part in text.split('/') {
        result.push(part);
        if fs::symlink_metadata(&result)
            .map_err(|e| error(format!("{}: {e}", result.display())))?
            .file_type()
            .is_symlink()
        {
            return Err(error("source path traverses a symlink"));
        }
    }
    Ok(result)
}

fn validate_values(values: &[Value]) -> Result<(), HarnessError> {
    let mut pending: Vec<(&Value, usize)> = values.iter().map(|v| (v, 0)).collect();
    let mut cells = 0;
    while let Some((value, depth)) = pending.pop() {
        cells += 1;
        if depth > 32 || cells > 262_144 {
            return Err(error("test value exceeds depth/cell cap"));
        }
        match value {
            Value::Record(v) => pending.extend(v.iter().map(|v| (v, depth + 1))),
            Value::List(v) => pending.extend(v.iter().map(|v| (v, depth + 1))),
            Value::Variant(_, Some(v)) => pending.push((v, depth + 1)),
            _ => {}
        }
    }
    Ok(())
}

/// Read a strict catalog and retain the exact checked source bytes for the run.
/// All paths and cases are validated, including cases later ignored/filtered.
pub fn load(path: &Path) -> Result<Suite, HarnessError> {
    let bytes = read_file(path, MAX_CATALOG)?;
    let identity = CatalogIdentity {
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: bytes.len(),
    };
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    let json = UniqueJson::deserialize(&mut decoder).map_err(|e| error(e.to_string()))?;
    decoder.end().map_err(|e| error(e.to_string()))?;
    let catalog: Catalog = serde_json::from_value(json.0).map_err(|e| error(e.to_string()))?;
    if catalog.format != "fluxr-tests-v1" {
        return Err(error("unsupported test catalog format"));
    }
    if catalog.tests.len() > MAX_TESTS {
        return Err(error("too many test cases"));
    }
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| error(e.to_string()))?;
    let mut names = BTreeSet::new();
    let mut sources = BTreeMap::new();
    let mut total = 0usize;
    for case in &catalog.tests {
        if !text_valid(&case.name, 256) || !names.insert(case.name.clone()) {
            return Err(error("test names must be nonempty, bounded and unique"));
        }
        if !matches!(case.expect, Expectation::CompileError { .. })
            && !text_valid(&case.export, 256)
        {
            return Err(error(format!(
                "{}: test export must be nonempty",
                case.name
            )));
        }
        if case.export.len() > 256 || case.export.chars().any(char::is_control) {
            return Err(error("invalid test export"));
        }
        match &case.expect {
            Expectation::Error { error: e } | Expectation::CompileError { compile_error: e }
                if !text_valid(e, 4096) =>
            {
                return Err(error(
                    "error expectations require a nonempty bounded substring",
                ));
            }
            Expectation::Value { value } => validate_values(std::slice::from_ref(value))?,
            _ => {}
        }
        if case.ignore.as_ref().is_some_and(|s| !text_valid(s, 4096)) {
            return Err(error("ignored tests require a nonempty bounded reason"));
        }
        if case.arguments.len() > 64 || case.inputs.len() > 64 {
            return Err(error("too many test arguments or inputs"));
        }
        validate_values(&case.arguments)?;
        validate_values(&case.inputs)?;
        let l = case.limits;
        if l.instructions > 1_000_000 || l.cells > 262_144 || l.frames > 256 {
            return Err(error("test resource limits exceed harness caps"));
        }
        if let Some(p) = &case.property {
            if !(1..=1024).contains(&p.cases)
                || p.generators.len() > 64
                || !case.arguments.is_empty()
                || !matches!(
                    case.expect,
                    Expectation::Value {
                        value: Value::Bool(true)
                    }
                )
            {
                return Err(error(
                    "properties require 1..1024 cases, at most 64 generators, no fixed arguments and expected bool true",
                ));
            }
            let mut generated_cells = 0usize;
            for generator in &p.generators {
                match generator {
                    Generator::I64 { min, max } | Generator::I64List { min, max, .. }
                        if min > max =>
                    {
                        return Err(error("invalid signed generator range"));
                    }
                    Generator::U64 { min, max } | Generator::U64List { min, max, .. }
                        if min > max =>
                    {
                        return Err(error("invalid unsigned generator range"));
                    }
                    Generator::BoolList { min_len, max_len }
                    | Generator::I64List {
                        min_len, max_len, ..
                    }
                    | Generator::U64List {
                        min_len, max_len, ..
                    } if min_len > max_len || *max_len > MAX_LIST_LENGTH => {
                        return Err(error(
                            "list generator lengths must satisfy min_len <= max_len <= 4096",
                        ));
                    }
                    _ => {}
                }
                generated_cells = generated_cells
                    .checked_add(max_generated_cells(generator))
                    .filter(|&cells| cells <= MAX_GENERATED_CELLS)
                    .ok_or_else(|| error("generated arguments exceed 262,144 cell cap"))?;
            }
        }
        if !sources.contains_key(&case.source) {
            let file = source_path(&root, &case.source)?;
            let bytes = read_file(&file, MAX_SOURCE)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| error("source byte overflow"))?;
            if total > MAX_SOURCE_TOTAL {
                return Err(error("combined source exceeds 32 MiB"));
            }
            sources.insert(
                case.source.clone(),
                String::from_utf8(bytes).map_err(|e| error(e.to_string()))?,
            );
        }
    }
    Ok(Suite {
        catalog,
        identity,
        sources,
    })
}

enum Compiled {
    Module(Module),
    Rejected(String),
    Unsupported(String),
}
fn compile(source: &str) -> Compiled {
    let document = match Document::parse(source) {
        Ok(d) => d,
        Err(e) => return Compiled::Rejected(e.to_string()),
    };
    if document.has_native_items() {
        return Compiled::Unsupported(
            "checked test catalogs cannot execute native source spans; use Cargo/libtest".into(),
        );
    }
    match document.compile() {
        Ok(m) => Compiled::Module(m),
        Err(e) => Compiled::Rejected(e.to_string()),
    }
}

fn selected(case: &TestCase, options: &Options) -> bool {
    let matches = |filter: &str| {
        if options.exact {
            case.name == filter
        } else {
            case.name.contains(filter)
        }
    };
    options.filter.as_deref().is_none_or(matches)
        && !options.skip.iter().any(|s| matches(s))
        && (!options.ignored_only || case.ignore.is_some())
}
fn ignored(case: &TestCase, options: &Options) -> bool {
    case.ignore.is_some() && !options.include_ignored && !options.ignored_only
}

/// Execute selected cases. The default is one worker; 1..8 workers are allowed.
/// Results always follow catalog order. With parallel fail-fast, already-started
/// cases finish; which cases started before the failure is intentionally not a
/// deterministic scheduling guarantee. No started/unfinished test is a pass.
pub fn run(suite: &Suite, options: &Options) -> Result<Report, HarnessError> {
    if !(1..=100).contains(&options.repeat)
        || !(1..=8).contains(&options.test_threads)
        || (options.ignored_only && options.include_ignored)
    {
        return Err(error(
            "repeat must be 1..100, test threads 1..8, and ignored modes exclusive",
        ));
    }
    let cases: Vec<&TestCase> = suite
        .catalog
        .tests
        .iter()
        .filter(|c| selected(c, options))
        .collect();
    if cases.is_empty() && !options.allow_empty {
        return Err(error("no tests selected; use --allow-empty to permit this"));
    }
    let sources = suite
        .sources
        .iter()
        .map(|(path, source)| SourceIdentity {
            path: path.clone(),
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            bytes: source.len(),
        })
        .collect();
    let mut report = Report {
        format: "fluxr-test-results-v1",
        catalog: suite.identity.clone(),
        configuration: options.clone(),
        success: true,
        listed: options.list,
        selected: cases.len(),
        passed: 0,
        failed: 0,
        ignored: 0,
        filtered: suite.catalog.tests.len() - cases.len(),
        not_run: 0,
        sources,
        tests: vec![],
    };
    if options.list {
        report.tests = cases
            .iter()
            .map(|c| empty_result(c, Status::Listed, c.ignore.clone()))
            .collect();
        return Ok(report);
    }
    let mut calls = 0u64;
    let mut fuel = 0u64;
    let mut generated_cell_work = 0u64;
    let multiplier = if options.mode == Mode::Compare { 2 } else { 1 };
    for case in &cases {
        if ignored(case, options) {
            continue;
        }
        let count = case
            .property
            .as_ref()
            .map_or(1, |p| p.cases + if p.shrink { MAX_SHRINK } else { 0 });
        let count = count as u64 * options.repeat as u64 * multiplier;
        calls += count;
        fuel += count * case.limits.instructions;
        if let Some(property) = &case.property {
            let cells: u64 = property
                .generators
                .iter()
                .map(|generator| max_generated_cells(generator) as u64)
                .sum();
            // List generation and retained replay payloads share an aggregate
            // cell-work budget. Use the same conservative reservation
            // as calls, including repetitions, comparisons and shrink attempts.
            generated_cell_work += count * cells;
        }
    }
    if calls > MAX_CALLS || fuel > MAX_FUEL {
        return Err(error(
            "selected run exceeds 65,536 invocation or one-billion instruction reservation",
        ));
    }
    if generated_cell_work > MAX_GENERATED_CELL_WORK {
        return Err(error(
            "selected run exceeds 4,194,304 generated argument cell-work reservation",
        ));
    }
    let compiled: BTreeMap<String, Compiled> = cases
        .iter()
        .filter(|c| !ignored(c, options))
        .map(|c| c.source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|path| {
            let value = compile(&suite.sources[&path]);
            (path, value)
        })
        .collect();
    let results: Mutex<Vec<Option<TestResult>>> = Mutex::new(vec![None; cases.len()]);
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..options.test_threads.min(cases.len()) {
            scope.spawn(|| {
                loop {
                    if options.fail_fast && stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(index) else {
                        break;
                    };
                    let result = if ignored(case, options) {
                        empty_result(case, Status::Ignored, case.ignore.clone())
                    } else {
                        run_case(case, &compiled[&case.source], options)
                    };
                    if matches!(result.status, Status::Failed) {
                        stopped.store(true, Ordering::Release);
                    }
                    results.lock().expect("test result lock poisoned")[index] = Some(result);
                }
            });
        }
    });
    report.tests = results
        .into_inner()
        .map_err(|_| error("test result lock poisoned"))?
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            result.unwrap_or_else(|| {
                let case = cases[index];
                if ignored(case, options) {
                    empty_result(case, Status::Ignored, case.ignore.clone())
                } else {
                    empty_result(
                        case,
                        Status::NotRun,
                        Some("not started after fail-fast".into()),
                    )
                }
            })
        })
        .collect();
    for result in &report.tests {
        match result.status {
            Status::Passed => report.passed += 1,
            Status::Failed => report.failed += 1,
            Status::Ignored => report.ignored += 1,
            Status::NotRun => report.not_run += 1,
            Status::Listed => {}
        }
    }
    report.success = report.failed == 0 && report.not_run == 0;
    Ok(report)
}

fn empty_result(case: &TestCase, status: Status, detail: Option<String>) -> TestResult {
    TestResult {
        name: case.name.clone(),
        status,
        executions: 0,
        routes: vec![],
        detail,
        replay: None,
    }
}
fn bounded(text: impl Into<String>) -> String {
    let mut text = text.into();
    if text.len() > 4096 {
        let mut end = 4096;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(" …");
    }
    text
}
struct Failure {
    message: String,
    signature: String,
}
fn failure(message: impl Into<String>, signature: impl Into<String>) -> Failure {
    Failure {
        message: bounded(message),
        signature: signature.into(),
    }
}

struct Executor<'a> {
    module: &'a Module,
    prepared: Option<PreparedCall<'a>>,
    portable_workspace: Workspace,
    prepared_workspace: Workspace,
    routes: BTreeSet<String>,
    executions: usize,
}
impl<'a> Executor<'a> {
    fn call(&mut self, case: &TestCase, arguments: &[Value], mode: Mode) -> Result<(), Failure> {
        self.executions += 1;
        let mut portable = None;
        if mode != Mode::Prepared {
            self.routes.insert("Portable".into());
            portable = Some(
                self.module
                    .run_portable_with(
                        Invocation {
                            export: &case.export,
                            inputs: &case.inputs,
                            arguments,
                            limits: case.limits,
                        },
                        &mut self.portable_workspace,
                    )
                    .map_err(|e| e.to_string()),
            );
        }
        let actual = if mode != Mode::Portable {
            let (actual, route) = self
                .prepared
                .as_ref()
                .expect("prepared handle selected")
                .run_with_route(
                    &case.inputs,
                    arguments,
                    case.limits,
                    &mut self.prepared_workspace,
                );
            self.routes.insert(format!("{route:?}"));
            let actual = actual.map_err(|e| e.to_string());
            if let Some(portable) = &portable {
                if portable != &actual {
                    return Err(failure(
                        format!(
                            "portable/prepared mismatch: portable={portable:?}, prepared={actual:?}"
                        ),
                        "backend_mismatch",
                    ));
                }
            }
            actual
        } else {
            portable.expect("portable outcome selected")
        };
        match (&case.expect, actual) {
            (Expectation::Value { value }, Ok(actual)) if value == &actual => Ok(()),
            (Expectation::Error { error }, Err(actual)) if actual.contains(error) => Ok(()),
            (Expectation::Value { value }, Ok(actual)) => Err(failure(
                format!("expected value {value:?}, got {actual:?}"),
                "value_mismatch",
            )),
            (Expectation::Value { .. }, Err(actual)) => Err(failure(
                format!("unexpected runtime error: {actual}"),
                format!("runtime:{actual}"),
            )),
            (Expectation::Error { error }, Ok(actual)) => Err(failure(
                format!("expected runtime error containing {error:?}, got value {actual:?}"),
                "missing_runtime_error",
            )),
            (Expectation::Error { error }, Err(actual)) => Err(failure(
                format!("expected runtime error containing {error:?}, got {actual:?}"),
                format!("runtime:{actual}"),
            )),
            (Expectation::CompileError { .. }, _) => {
                unreachable!("compile expectations handled before calls")
            }
        }
    }
}

fn run_case(case: &TestCase, compiled: &Compiled, options: &Options) -> TestResult {
    let mut result = empty_result(case, Status::Failed, None);
    let module = match (compiled, &case.expect) {
        (Compiled::Rejected(actual), Expectation::CompileError { compile_error })
            if actual.contains(compile_error) =>
        {
            result.status = Status::Passed;
            return result;
        }
        (Compiled::Rejected(actual), _) => {
            result.detail = Some(bounded(format!("compilation rejected: {actual}")));
            return result;
        }
        (Compiled::Unsupported(actual), _) => {
            result.detail = Some(actual.clone());
            return result;
        }
        (Compiled::Module(_), Expectation::CompileError { .. }) => {
            result.detail = Some("expected compilation failure but source compiled".into());
            return result;
        }
        (Compiled::Module(module), _) => module,
    };
    let Some((params, return_type)) = module.export_signatures().get(&case.export) else {
        result.detail = Some(format!("harness: unknown export {}", case.export));
        return result;
    };
    if let Some(property) = &case.property {
        if return_type != &Type::Bool
            || params.len() != property.generators.len()
            || !params
                .iter()
                .zip(&property.generators)
                .all(|(ty, g)| match (ty, g) {
                    (Type::Bool, Generator::Bool {})
                    | (Type::I64, Generator::I64 { .. })
                    | (Type::U64, Generator::U64 { .. }) => true,
                    (Type::List(element), Generator::BoolList { .. }) => **element == Type::Bool,
                    (Type::List(element), Generator::I64List { .. }) => **element == Type::I64,
                    (Type::List(element), Generator::U64List { .. }) => **element == Type::U64,
                    _ => false,
                })
        {
            result.detail =
                Some("harness: property generators must match the Boolean export signature".into());
            return result;
        }
    }
    let prepared = if options.mode != Mode::Portable {
        match module.prepare_call(&case.export) {
            Ok(p) => Some(p),
            Err(e) => {
                result.detail = Some(format!("harness: prepare failed: {e}"));
                return result;
            }
        }
    } else {
        None
    };
    let mut executor = Executor {
        module,
        prepared,
        portable_workspace: Workspace::default(),
        prepared_workspace: Workspace::default(),
        routes: BTreeSet::new(),
        executions: 0,
    };
    'repetitions: for repetition in 0..options.repeat {
        let mut random = case.property.as_ref().map_or(0, |p| p.seed);
        for index in 0..case.property.as_ref().map_or(1, |p| p.cases) {
            let arguments = case.property.as_ref().map_or_else(
                || case.arguments.clone(),
                |p| {
                    p.generators
                        .iter()
                        .map(|g| generate(g, &mut random))
                        .collect()
                },
            );
            if let Err(failed) = executor.call(case, &arguments, options.mode) {
                result.detail = Some(failed.message);
                if let Some(property) = &case.property {
                    let (shrunk, evaluations) = if property.shrink {
                        shrink(
                            case,
                            property,
                            &arguments,
                            &failed.signature,
                            &mut executor,
                            options.mode,
                        )
                    } else {
                        (arguments.clone(), 0)
                    };
                    result.replay = Some(Replay {
                        seed: property.seed,
                        case: index,
                        repetition,
                        arguments,
                        shrunk_arguments: shrunk,
                        shrink_evaluations: evaluations,
                    });
                }
                break 'repetitions;
            }
        }
    }
    if result.detail.is_none() {
        result.status = Status::Passed;
    }
    result.executions = executor.executions;
    result.routes = executor.routes.into_iter().collect();
    result
}

fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
fn generate(generator: &Generator, state: &mut u64) -> Value {
    // Preserve the released scalar stream, including singleton ranges. Lists
    // consume this draw for length, even when fixed or empty, then one per cell.
    let draw = random(state) as u128;
    match *generator {
        Generator::Bool {} => Value::Bool(draw & 1 != 0),
        Generator::I64 { min, max } => Value::I64(
            (min as i128 + (draw % (max as i128 - min as i128 + 1) as u128) as i128) as i64,
        ),
        Generator::U64 { min, max } => {
            Value::U64((min as u128 + draw % (max as u128 - min as u128 + 1)) as u64)
        }
        Generator::BoolList { min_len, max_len } => {
            generate_list(min_len, max_len, &Generator::Bool {}, draw, state)
        }
        Generator::I64List {
            min_len,
            max_len,
            min,
            max,
        } => generate_list(min_len, max_len, &Generator::I64 { min, max }, draw, state),
        Generator::U64List {
            min_len,
            max_len,
            min,
            max,
        } => generate_list(min_len, max_len, &Generator::U64 { min, max }, draw, state),
    }
}

fn generate_list(
    min_len: usize,
    max_len: usize,
    element: &Generator,
    draw: u128,
    state: &mut u64,
) -> Value {
    // load() admits these bounds and the aggregate maximum before generation.
    let length = min_len + (draw % (max_len - min_len + 1) as u128) as usize;
    Value::List(Arc::new(
        (0..length).map(|_| generate(element, state)).collect(),
    ))
}

fn scalar_candidates(value: &Value, generator: &Generator) -> Vec<Value> {
    match (value, generator) {
        (Value::Bool(true), Generator::Bool {}) => vec![Value::Bool(false)],
        (Value::I64(value), Generator::I64 { min, max }) => {
            let target = 0i64.clamp(*min, *max);
            let midpoint = (*value as i128 + (target as i128 - *value as i128) / 2) as i64;
            vec![Value::I64(target), Value::I64(midpoint)]
        }
        (Value::U64(value), Generator::U64 { min, .. }) => {
            vec![Value::U64(*min), Value::U64(*min + (*value - *min) / 2)]
        }
        _ => vec![],
    }
}

struct Shrinker<'a, 'module> {
    case: &'a TestCase,
    signature: &'a str,
    executor: &'a mut Executor<'module>,
    mode: Mode,
    current: Vec<Value>,
    evaluations: usize,
}

impl Shrinker<'_, '_> {
    fn try_value(&mut self, index: usize, value: Value) -> bool {
        if self.evaluations >= MAX_SHRINK || value == self.current[index] {
            return false;
        }
        let mut proposed = self.current.clone();
        proposed[index] = value;
        self.evaluations += 1;
        if self
            .executor
            .call(self.case, &proposed, self.mode)
            .is_err_and(|failed| failed.signature == self.signature)
        {
            self.current = proposed;
            true
        } else {
            false
        }
    }

    fn scalar(&mut self, index: usize, generator: &Generator) {
        loop {
            if self.evaluations >= MAX_SHRINK {
                return;
            }
            let candidates = scalar_candidates(&self.current[index], generator);
            let mut changed = false;
            for value in candidates {
                if self.try_value(index, value) {
                    changed = true;
                    break;
                }
            }
            if !changed {
                return;
            }
        }
    }

    fn list(&mut self, index: usize, min_len: usize, element: &Generator) {
        // Delete the largest allowed contiguous chunk first, visiting offsets
        // left to right. Halve rejected chunk sizes; restart after acceptance.
        // Only one candidate list is allocated at a time.
        'lengths: loop {
            if self.evaluations >= MAX_SHRINK {
                return;
            }
            let Value::List(values) = &self.current[index] else {
                return;
            };
            let length = values.len();
            let mut chunk = length - min_len;
            while chunk > 0 {
                for offset in 0..=length - chunk {
                    if self.evaluations >= MAX_SHRINK {
                        return;
                    }
                    let Value::List(values) = &self.current[index] else {
                        return;
                    };
                    let mut reduced = Vec::with_capacity(length - chunk);
                    reduced.extend_from_slice(&values[..offset]);
                    reduced.extend_from_slice(&values[offset + chunk..]);
                    if self.try_value(index, Value::List(Arc::new(reduced))) {
                        continue 'lengths;
                    }
                }
                chunk /= 2;
            }
            break;
        }

        let Value::List(values) = &self.current[index] else {
            return;
        };
        for cell in 0..values.len() {
            loop {
                if self.evaluations >= MAX_SHRINK {
                    return;
                }
                let Value::List(values) = &self.current[index] else {
                    return;
                };
                let candidates = scalar_candidates(&values[cell], element);
                let mut changed = false;
                for value in candidates {
                    if self.evaluations >= MAX_SHRINK {
                        return;
                    }
                    let Value::List(values) = &self.current[index] else {
                        return;
                    };
                    if value == values[cell] {
                        continue;
                    }
                    let mut reduced = values.as_ref().clone();
                    reduced[cell] = value;
                    if self.try_value(index, Value::List(Arc::new(reduced))) {
                        changed = true;
                        break;
                    }
                }
                if !changed {
                    break;
                }
            }
        }
    }
}

fn shrink(
    case: &TestCase,
    property: &Property,
    original: &[Value],
    signature: &str,
    executor: &mut Executor<'_>,
    mode: Mode,
) -> (Vec<Value>, usize) {
    let mut shrinker = Shrinker {
        case,
        signature,
        executor,
        mode,
        current: original.to_vec(),
        evaluations: 0,
    };
    for (index, generator) in property.generators.iter().enumerate() {
        match *generator {
            Generator::BoolList { min_len, .. } => {
                shrinker.list(index, min_len, &Generator::Bool {});
            }
            Generator::I64List {
                min_len, min, max, ..
            } => shrinker.list(index, min_len, &Generator::I64 { min, max }),
            Generator::U64List {
                min_len, min, max, ..
            } => shrinker.list(index, min_len, &Generator::U64 { min, max }),
            _ => shrinker.scalar(index, generator),
        }
    }
    (shrinker.current, shrinker.evaluations)
}

pub const USAGE: &str = "usage: fluxr-cli test-suite CATALOG [FILTER] [--exact] [--skip FILTER] [--list]\n       [--ignored | --include-ignored] [--fail-fast] [--allow-empty]\n       [--repeat 1..100] [--test-threads 1..8] [--format text|json]\n       [--mode portable|prepared|compare]\nCatalog: fluxr-tests-v1; source paths are relative to the catalog.\nProperties retain their seed and failure inputs; shrink attempts are bounded.";

/// CLI entry; returns false only for an executed suite containing failures.
pub fn run_cli(args: &[String]) -> Result<bool, HarnessError> {
    if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
        return Ok(true);
    }
    let Some(path) = args.first().filter(|p| !p.starts_with('-')) else {
        return Err(error(USAGE));
    };
    let mut options = Options::default();
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        let mut value = || {
            index += 1;
            args.get(index)
                .ok_or_else(|| error(format!("missing value for {arg}")))
        };
        match arg.as_str() {
            "--exact" => options.exact = true,
            "--list" => options.list = true,
            "--ignored" => options.ignored_only = true,
            "--include-ignored" => options.include_ignored = true,
            "--fail-fast" => options.fail_fast = true,
            "--allow-empty" => options.allow_empty = true,
            "--skip" => options.skip.push(value()?.clone()),
            "--repeat" => options.repeat = value()?.parse().map_err(|_| error("invalid repeat"))?,
            "--test-threads" => {
                options.test_threads = value()?
                    .parse()
                    .map_err(|_| error("invalid test threads"))?
            }
            "--format" => {
                options.output = match value()?.as_str() {
                    "text" => Output::Text,
                    "json" => Output::Json,
                    _ => return Err(error("format must be text or json")),
                }
            }
            "--mode" => {
                options.mode = match value()?.as_str() {
                    "portable" => Mode::Portable,
                    "prepared" => Mode::Prepared,
                    "compare" => Mode::Compare,
                    _ => return Err(error("mode must be portable, prepared or compare")),
                }
            }
            _ if !arg.starts_with('-') && options.filter.is_none() => {
                options.filter = Some(arg.clone())
            }
            _ => return Err(error(format!("unknown test option: {arg}\n{USAGE}"))),
        }
        index += 1;
    }
    let suite = load(Path::new(path))?;
    let report = run(&suite, &options)?;
    let mut out = std::io::stdout().lock();
    if options.output == Output::Json {
        serde_json::to_writer(&mut out, &report).map_err(|e| error(e.to_string()))?;
        writeln!(out).map_err(|e| error(e.to_string()))?;
    } else {
        for result in &report.tests {
            writeln!(
                out,
                "{} ... {:?}{}",
                result.name,
                result.status,
                result
                    .detail
                    .as_ref()
                    .map_or(String::new(), |d| format!(": {d}"))
            )
            .map_err(|e| error(e.to_string()))?;
            if let Some(replay) = &result.replay {
                writeln!(
                    out,
                    "  replay: {}",
                    serde_json::to_string(replay).map_err(|e| error(e.to_string()))?
                )
                .map_err(|e| error(e.to_string()))?;
            }
        }
        writeln!(
            out,
            "{} passed; {} failed; {} ignored; {} filtered; {} not run{}",
            report.passed,
            report.failed,
            report.ignored,
            report.filtered,
            report.not_run,
            if report.listed { " (listing only)" } else { "" }
        )
        .map_err(|e| error(e.to_string()))?;
    }
    Ok(report.success)
}

#[cfg(test)]
mod generator_tests {
    use super::{Generator, generate};
    use crate::structured::Value;

    #[test]
    fn singleton_scalar_domains_preserve_released_draw_consumption() {
        let mut state = 17;
        assert_eq!(
            generate(&Generator::I64 { min: -7, max: -7 }, &mut state),
            Value::I64(-7)
        );
        assert_eq!(
            generate(&Generator::U64 { min: 11, max: 11 }, &mut state),
            Value::U64(11)
        );
        assert_eq!(
            generate(&Generator::Bool {}, &mut state),
            Value::Bool(false)
        );
        assert_eq!(
            generate(
                &Generator::U64 {
                    min: 0,
                    max: u64::MAX
                },
                &mut state
            ),
            Value::U64(4_130_155_771_025_092_611)
        );
        assert_eq!(state, 8_709_371_129_873_690_725);
    }

    #[test]
    fn every_empty_list_still_consumes_one_length_draw() {
        let mut state = 17;
        for generator in [
            Generator::BoolList {
                min_len: 0,
                max_len: 0,
            },
            Generator::I64List {
                min_len: 0,
                max_len: 0,
                min: i64::MIN,
                max: i64::MAX,
            },
            Generator::U64List {
                min_len: 0,
                max_len: 0,
                min: 0,
                max: u64::MAX,
            },
        ] {
            assert_eq!(generate(&generator, &mut state), Value::List(vec![].into()));
        }
        assert_eq!(
            generate(
                &Generator::U64 {
                    min: 0,
                    max: u64::MAX
                },
                &mut state
            ),
            Value::U64(4_130_155_771_025_092_611)
        );
        assert_eq!(state, 8_709_371_129_873_690_725);
    }

    #[test]
    fn fixed_list_lengths_and_singleton_elements_keep_their_draws() {
        let mut state = 17;
        assert_eq!(
            generate(
                &Generator::BoolList {
                    min_len: 1,
                    max_len: 1
                },
                &mut state
            ),
            Value::List(vec![Value::Bool(true)].into())
        );
        assert_eq!(
            generate(
                &Generator::I64List {
                    min_len: 2,
                    max_len: 2,
                    min: -7,
                    max: -7
                },
                &mut state,
            ),
            Value::List(vec![Value::I64(-7), Value::I64(-7)].into())
        );
        assert_eq!(
            generate(
                &Generator::U64List {
                    min_len: 1,
                    max_len: 1,
                    min: 0,
                    max: u64::MAX
                },
                &mut state,
            ),
            Value::List(vec![Value::U64(578_614_372_503_024_644)].into())
        );
        assert_eq!(
            generate(
                &Generator::U64 {
                    min: 0,
                    max: u64::MAX
                },
                &mut state
            ),
            Value::U64(12_055_495_660_123_309_877)
        );
        assert_eq!(state, 17_418_742_259_747_381_433);
    }
}
