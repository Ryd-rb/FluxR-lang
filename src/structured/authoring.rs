//! AH1: immutable typed body holes over an exactly bound, checked FA1 base.
//!
//! A draft is analysis-only. Selected bodies are erased, and no `Module` is
//! retained or exposed. Filling uses the existing checker; only final full
//! compilation admits a completed candidate. Hashes bind observations but do
//! not authenticate the caller's compiler identity or authorize publication.
use super::{Binding, BodyChecker, Function, Module, Source, Stmt, Type};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const PROFILE: &str = "fluxr-ah1-v1";
/// Opt-in model-facing projection; the host-retained binding stays AH1.
pub const COMPACT_CONTEXT_PROFILE: &str = "fluxr-ac1-v1";
pub const COMPILER_CONTRACT: &str = "structured-fsm-v1-v2-fa1";
const MAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_HOLES: usize = 512;
const MAX_DETAIL_BYTES: usize = 240;
type Result<T> = std::result::Result<T, AuthoringError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidRequest,
    Parse,
    Type,
    Stale,
    UnknownTarget,
    AmbiguousTarget,
    Unresolved,
    AlreadyFilled,
    Cancelled,
    OutputLimit,
    ResourceLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthoringError {
    pub kind: ErrorKind,
    detail: String,
}

impl AuthoringError {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for AuthoringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AH1 {:?}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for AuthoringError {}

fn fail(kind: ErrorKind, detail: impl fmt::Display) -> AuthoringError {
    let mut detail = detail.to_string();
    let mut end = detail.len().min(MAX_DETAIL_BYTES);
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail.truncate(end);
    AuthoringError { kind, detail }
}

/// Terminal request cancellation. Clones refer to the same request; create a
/// fresh token for another independent request. It cannot be reset or revived.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(fail(ErrorKind::Cancelled, "request cancelled"))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct BodyTarget {
    pub receiver: Option<String>,
    pub name: String,
}

impl BodyTarget {
    pub fn function(name: impl Into<String>) -> Self {
        Self {
            receiver: None,
            name: name.into(),
        }
    }

    pub fn method(receiver: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            receiver: Some(receiver.into()),
            name: name.into(),
        }
    }
}

/// Exact binding for an analysis state, not an executable artifact identity.
/// `compiler_sha256` is supplied by the trusted host and is not authenticated by
/// AH1. Source bytes, semantic base, declaration index and sketch are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DraftBinding {
    pub profile: &'static str,
    pub compiler_contract: &'static str,
    pub compiler_sha256: [u8; 32],
    pub buffer_version: u64,
    pub generation: u64,
    pub source_sha256: [u8; 32],
    pub base_semantic_sha256: [u8; 32],
    pub node_index_sha256: [u8; 32],
    pub sketch_sha256: [u8; 32],
    pub draft_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
pub struct QueryLimits {
    pub max_bytes: usize,
    pub max_items: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1024,
            max_items: 128,
        }
    }
}

/// Exact optional signature retrieval, not a semantic dependency claim. Empty
/// explicit lists request no optional signatures; current scope is mandatory.
/// All selectors are checked before output budgets can truncate the response.
#[derive(Clone, Copy, Debug)]
pub enum SignatureSelection<'a> {
    All,
    Explicit {
        records: &'a [&'a str],
        enums: &'a [&'a str],
        callables: &'a [BodyTarget],
    },
}

/// A host-retained query ticket plus a bounded model-facing packet. There is no
/// deserializer: a model's invented context cannot replace this ticket. A cloned
/// ticket is usable only against its identical immutable draft binding.
#[derive(Clone, Debug)]
pub struct BodyContext {
    binding: DraftBinding,
    target: BodyTarget,
    index: usize,
    expected: Type,
    packet: String,
    visible_items: usize,
    selected_items: usize,
    total_items: usize,
    truncated: bool,
    packet_profile: &'static str,
}

impl BodyContext {
    pub fn binding(&self) -> &DraftBinding {
        &self.binding
    }
    pub fn target(&self) -> &BodyTarget {
        &self.target
    }
    pub fn expected_type(&self) -> &Type {
        &self.expected
    }
    pub fn packet(&self) -> &str {
        &self.packet
    }
    pub fn visible_items(&self) -> usize {
        self.visible_items
    }
    /// Mandatory scope plus requested optional signatures, before truncation.
    pub fn selected_items(&self) -> usize {
        self.selected_items
    }
    /// Inventory omitted by explicit selection, distinct from output truncation.
    pub fn unselected_items(&self) -> usize {
        self.total_items - self.selected_items
    }
    pub fn total_items(&self) -> usize {
        self.total_items
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn analysis_only(&self) -> bool {
        true
    }
    pub fn packet_profile(&self) -> &'static str {
        self.packet_profile
    }
}

#[derive(Clone)]
pub struct AuthoringDraft {
    original: Arc<str>,
    sketch: Arc<Source>,
    holes: BTreeSet<usize>,
    binding: DraftBinding,
}

impl AuthoringDraft {
    /// Open a strict checked FA1 base and erase all selected bodies. This is not
    /// a recovery parser for arbitrary malformed source. No old body remains in
    /// the sketch as an executable placeholder for a hole.
    pub fn open_fa1(
        text: &str,
        buffer_version: u64,
        compiler_sha256: [u8; 32],
        holes: &[BodyTarget],
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        if text.len() > MAX_SOURCE_BYTES || holes.len() > MAX_HOLES {
            return Err(fail(
                ErrorKind::ResourceLimit,
                "source or hole limit exceeded",
            ));
        }
        if holes.is_empty() || compiler_sha256 == [0; 32] {
            return Err(fail(
                ErrorKind::InvalidRequest,
                "holes and compiler identity required",
            ));
        }
        let mut source = Source::parse_agent(text).map_err(|e| fail(ErrorKind::Parse, e))?;
        cancel.check()?;
        let checked = Module::compile_borrowed(&source).map_err(|e| fail(ErrorKind::Type, e))?;
        cancel.check()?;
        let source_sha256 = Sha256::digest(text.as_bytes()).into();
        let base_semantic_sha256 = Sha256::digest(checked.canonical_source()).into();
        let index: Vec<_> = source.functions.iter().map(target_of).collect();
        let node_index_sha256 = hash_serialized(&(PROFILE, source_sha256, &index))?;
        let mut unresolved = BTreeSet::new();
        for target in holes {
            cancel.check()?;
            let index = locate(&source, target)?;
            if !unresolved.insert(index) {
                return Err(fail(ErrorKind::InvalidRequest, "duplicate hole target"));
            }
        }
        for &index in &unresolved {
            source.functions[index].body.clear();
        }
        let binding = DraftBinding {
            profile: PROFILE,
            compiler_contract: COMPILER_CONTRACT,
            compiler_sha256,
            buffer_version,
            generation: 0,
            source_sha256,
            base_semantic_sha256,
            node_index_sha256,
            sketch_sha256: [0; 32],
            draft_sha256: [0; 32],
        };
        let mut draft = Self {
            original: Arc::from(text),
            sketch: Arc::new(source),
            holes: unresolved,
            binding,
        };
        draft.rebind()?;
        cancel.check()?;
        Ok(draft)
    }

    pub fn binding(&self) -> &DraftBinding {
        &self.binding
    }
    pub fn original_source(&self) -> &str {
        &self.original
    }
    pub fn unresolved_count(&self) -> usize {
        self.holes.len()
    }

    /// Shorthand is accepted only when the name has exactly one declaration.
    /// Explicit function/method targets remain available for overloaded names.
    pub fn resolve_name(&self, name: &str) -> Result<BodyTarget> {
        if !valid_name(name) {
            return Err(fail(ErrorKind::InvalidRequest, "invalid declaration name"));
        }
        let mut matches = self.sketch.functions.iter().filter(|f| f.name == name);
        let found = matches
            .next()
            .ok_or_else(|| fail(ErrorKind::UnknownTarget, "unknown declaration"))?;
        if matches.next().is_some() {
            return Err(fail(
                ErrorKind::AmbiguousTarget,
                "name needs an explicit receiver identity",
            ));
        }
        Ok(target_of(found))
    }

    /// Return only signatures and obligations, never another declaration's body.
    /// A truncated context is explicitly incomplete and remains analysis-only.
    pub fn query(
        &self,
        expected: &DraftBinding,
        target: &BodyTarget,
        limits: QueryLimits,
        cancel: &Cancellation,
    ) -> Result<BodyContext> {
        cancel.check()?;
        self.check_binding(expected)?;
        if limits.max_bytes == 0
            || limits.max_bytes > MAX_CONTEXT_BYTES
            || limits.max_items > MAX_ITEMS
        {
            return Err(fail(
                ErrorKind::InvalidRequest,
                "invalid query output limits",
            ));
        }
        let index = locate(&self.sketch, target)?;
        if !self.holes.contains(&index) {
            return Err(fail(
                ErrorKind::AlreadyFilled,
                "target is not an unresolved hole",
            ));
        }
        let function = &self.sketch.functions[index];
        // Reserve a constant upper bound for counters and the completion trailer.
        const TRAILER_RESERVE: usize = 100;
        let mut context = ContextBuilder {
            packet: header(
                &self.binding,
                target,
                index,
                &function.result,
                limits.max_bytes.saturating_sub(TRAILER_RESERVE),
            )?,
            visible: 0,
            total: 0,
            truncated: false,
            max_items: limits.max_items,
            cancel,
        };
        for (kind, bindings) in [
            ("param", &function.params),
            ("local", &function.locals),
            ("input", &self.sketch.inputs),
        ] {
            for binding in bindings {
                context.item(|line| {
                    write!(line, "{kind} {} ", binding.name)?;
                    write_type(line, &binding.ty)?;
                    line.write_char('\n')
                })?;
            }
        }
        for record in &self.sketch.records {
            context.item(|line| {
                write!(line, "rec {} {}", record.name, record.fields.len())?;
                append_bindings(line, &record.fields)?;
                line.write_char('\n')
            })?;
        }
        for enumeration in &self.sketch.enums {
            context.item(|line| {
                write!(
                    line,
                    "enum {} {}",
                    enumeration.name,
                    enumeration.variants.len()
                )?;
                for variant in &enumeration.variants {
                    write!(line, " {} ", variant.name)?;
                    match &variant.payload {
                        Some(kind) => write_type(line, kind)?,
                        None => line.write_str("none")?,
                    }
                }
                line.write_char('\n')
            })?;
        }
        for declaration in &self.sketch.functions {
            context.item(|line| {
                match &declaration.receiver {
                    Some(receiver) => write!(
                        line,
                        "method {receiver} {} {}",
                        declaration.name,
                        declaration.params.len()
                    )?,
                    None => write!(line, "fn {} {}", declaration.name, declaration.params.len())?,
                }
                append_bindings(line, &declaration.params)?;
                line.write_char(' ')?;
                write_type(line, &declaration.result)?;
                line.write_char('\n')
            })?;
        }
        context.packet.limit = limits.max_bytes;
        write!(
            context.packet,
            "items {} {}\ncontext {}\nadmission none\n",
            context.visible,
            context.total,
            if context.truncated {
                "truncated"
            } else {
                "signature_complete"
            }
        )
        .map_err(|_| {
            fail(
                if context.packet.allocation_failed {
                    ErrorKind::ResourceLimit
                } else {
                    ErrorKind::OutputLimit
                },
                "query trailer allocation or byte limit refused",
            )
        })?;
        cancel.check()?;
        Ok(BodyContext {
            binding: self.binding.clone(),
            target: target.clone(),
            index,
            expected: function.result.clone(),
            packet: context.packet.text,
            visible_items: context.visible,
            selected_items: context.total,
            total_items: context.total,
            truncated: context.truncated,
            packet_profile: PROFILE,
        })
    }

    /// AC1 projects one full draft digest and explicitly selected signatures to
    /// the model while retaining every AH1 binding component in this ticket.
    /// Apply a response with its originating ticket, never a newer query's
    /// ticket. Packet text alone cannot route, authorize or admit a body fill.
    pub fn query_compact(
        &self,
        expected: &DraftBinding,
        target: &BodyTarget,
        selection: SignatureSelection<'_>,
        limits: QueryLimits,
        cancel: &Cancellation,
    ) -> Result<BodyContext> {
        cancel.check()?;
        self.check_binding(expected)?;
        if limits.max_bytes == 0
            || limits.max_bytes > MAX_CONTEXT_BYTES
            || limits.max_items > MAX_ITEMS
        {
            return Err(fail(
                ErrorKind::InvalidRequest,
                "invalid query output limits",
            ));
        }
        let index = locate(&self.sketch, target)?;
        if !self.holes.contains(&index) {
            return Err(fail(
                ErrorKind::AlreadyFilled,
                "target is not an unresolved hole",
            ));
        }
        selection.validate(&self.sketch, cancel)?;
        let function = &self.sketch.functions[index];
        // Three bounded counters, selection status, completeness and admission.
        const TRAILER_RESERVE: usize = 160;
        let mut context = ContextBuilder {
            packet: compact_header(
                &self.binding,
                target,
                &function.result,
                limits.max_bytes.saturating_sub(TRAILER_RESERVE),
            )?,
            visible: 0,
            total: 0,
            truncated: false,
            max_items: limits.max_items,
            cancel,
        };
        let mut inventory = 0;
        for (kind, bindings) in [
            ("param", &function.params),
            ("local", &function.locals),
            ("input", &self.sketch.inputs),
        ] {
            for binding in bindings {
                inventory += 1;
                context.item(|line| {
                    write!(line, "{kind} {} ", binding.name)?;
                    write_type(line, &binding.ty)?;
                    line.write_char('\n')
                })?;
                if context.truncated {
                    return Err(fail(
                        ErrorKind::OutputLimit,
                        "mandatory current scope exceeds query byte or item limit",
                    ));
                }
            }
        }
        for record in &self.sketch.records {
            cancel.check()?;
            inventory += 1;
            if selection.record(&record.name) {
                context.item(|line| {
                    write!(line, "rec {} {}", record.name, record.fields.len())?;
                    append_bindings(line, &record.fields)?;
                    line.write_char('\n')
                })?;
            }
        }
        for enumeration in &self.sketch.enums {
            cancel.check()?;
            inventory += 1;
            if selection.enumeration(&enumeration.name) {
                context.item(|line| {
                    write!(
                        line,
                        "enum {} {}",
                        enumeration.name,
                        enumeration.variants.len()
                    )?;
                    for variant in &enumeration.variants {
                        write!(line, " {} ", variant.name)?;
                        match &variant.payload {
                            Some(kind) => write_type(line, kind)?,
                            None => line.write_str("none")?,
                        }
                    }
                    line.write_char('\n')
                })?;
            }
        }
        for declaration in &self.sketch.functions {
            cancel.check()?;
            inventory += 1;
            if selection.callable(declaration) {
                context.item(|line| {
                    match &declaration.receiver {
                        Some(receiver) => write!(
                            line,
                            "method {receiver} {} {}",
                            declaration.name,
                            declaration.params.len()
                        )?,
                        None => {
                            write!(line, "fn {} {}", declaration.name, declaration.params.len())?
                        }
                    }
                    append_bindings(line, &declaration.params)?;
                    line.write_char(' ')?;
                    write_type(line, &declaration.result)?;
                    line.write_char('\n')
                })?;
            }
        }
        context.packet.limit = limits.max_bytes;
        write!(
            context.packet,
            "items {} {} {}\nselection {}\ncontext {}\nadmission none\n",
            context.visible,
            context.total,
            inventory,
            match selection {
                SignatureSelection::All => "all",
                SignatureSelection::Explicit { .. } => "explicit",
            },
            if context.truncated {
                "truncated"
            } else {
                "selected_complete"
            }
        )
        .map_err(|_| {
            fail(
                if context.packet.allocation_failed {
                    ErrorKind::ResourceLimit
                } else {
                    ErrorKind::OutputLimit
                },
                "query trailer allocation or byte limit refused",
            )
        })?;
        cancel.check()?;
        Ok(BodyContext {
            binding: self.binding.clone(),
            target: target.clone(),
            index,
            expected: function.result.clone(),
            packet: context.packet.text,
            visible_items: context.visible,
            selected_items: context.total,
            total_items: inventory,
            truncated: context.truncated,
            packet_profile: COMPACT_CONTEXT_PROFILE,
        })
    }

    /// Fill exactly one typed body hole using FA1's existing body syntax. The
    /// operation is immutable and failure never changes either draft. Checking
    /// this body is not final whole-module instruction-budget admission.
    pub fn fill_body(
        &self,
        context: &BodyContext,
        text: &str,
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        self.check_binding(&context.binding)?;
        let index = locate(&self.sketch, &context.target)?;
        if index != context.index {
            return Err(fail(ErrorKind::Stale, "context declaration index differs"));
        }
        if !self.holes.contains(&index) {
            return Err(fail(
                ErrorKind::AlreadyFilled,
                "target is not an unresolved hole",
            ));
        }
        if text.len() > MAX_SOURCE_BYTES {
            return Err(fail(ErrorKind::ResourceLimit, "body byte limit exceeded"));
        }
        let body = Stmt::parse_agent_body(text).map_err(|e| fail(ErrorKind::Parse, e))?;
        cancel.check()?;
        let mut checker = BodyChecker::new(&self.sketch).map_err(|e| fail(ErrorKind::Type, e))?;
        checker
            .check(index, &body)
            .map_err(|e| fail(ErrorKind::Type, e))?;
        cancel.check()?;
        let mut source = (*self.sketch).clone();
        source.functions[index].body = body;
        let mut next = self.clone();
        next.sketch = Arc::new(source);
        next.holes.remove(&index);
        next.binding.generation = next
            .binding
            .generation
            .checked_add(1)
            .ok_or_else(|| fail(ErrorKind::ResourceLimit, "draft generation exhausted"))?;
        next.rebind()?;
        cancel.check()?;
        Ok(next)
    }

    /// Strict executable admission: every hole must have been explicitly filled,
    /// followed by the unchanged complete structured checker and module budget.
    pub fn admit(&self, expected: &DraftBinding, cancel: &Cancellation) -> Result<Module> {
        cancel.check()?;
        self.check_binding(expected)?;
        if !self.holes.is_empty() {
            return Err(fail(
                ErrorKind::Unresolved,
                "unresolved bodies forbid executable admission",
            ));
        }
        let module =
            Module::compile_borrowed(&self.sketch).map_err(|e| fail(ErrorKind::Type, e))?;
        cancel.check()?;
        Ok(module)
    }

    fn check_binding(&self, expected: &DraftBinding) -> Result<()> {
        if expected != &self.binding {
            Err(fail(
                ErrorKind::Stale,
                "source revision profile compiler or sketch binding differs",
            ))
        } else {
            Ok(())
        }
    }

    fn rebind(&mut self) -> Result<()> {
        let bytes = super::compile::encode_source(&self.sketch)
            .map_err(|e| fail(ErrorKind::ResourceLimit, e))?;
        self.binding.sketch_sha256 = Sha256::digest(bytes).into();
        self.binding.draft_sha256 = [0; 32];
        self.binding.draft_sha256 = hash_serialized(&(PROFILE, &self.binding, &self.holes))?;
        Ok(())
    }
}

fn hash_serialized(value: &impl Serialize) -> Result<[u8; 32]> {
    struct Writer(Sha256);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| fail(ErrorKind::InvalidRequest, e))?;
    Ok(writer.0.finalize().into())
}

fn target_of(function: &Function) -> BodyTarget {
    BodyTarget {
        receiver: function.receiver.clone(),
        name: function.name.clone(),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

fn locate(source: &Source, target: &BodyTarget) -> Result<usize> {
    if !valid_name(&target.name) || target.receiver.as_ref().is_some_and(|x| !valid_name(x)) {
        return Err(fail(ErrorKind::InvalidRequest, "invalid target identity"));
    }
    source
        .functions
        .iter()
        .position(|f| f.name == target.name && f.receiver == target.receiver)
        .ok_or_else(|| fail(ErrorKind::UnknownTarget, "unknown explicit target"))
}

impl SignatureSelection<'_> {
    fn validate(&self, source: &Source, cancel: &Cancellation) -> Result<()> {
        let Self::Explicit {
            records,
            enums,
            callables,
        } = self
        else {
            return Ok(());
        };
        if records
            .len()
            .saturating_add(enums.len())
            .saturating_add(callables.len())
            > MAX_ITEMS
        {
            return Err(fail(
                ErrorKind::ResourceLimit,
                "signature selection limit exceeded",
            ));
        }
        // Borrowed bounded selectors; validation cannot allocate an unbounded
        // index. Checked source declaration bounds also bound each scan.
        for (names, is_record) in [(*records, true), (*enums, false)] {
            for (index, name) in names.iter().enumerate() {
                cancel.check()?;
                if !valid_name(name) || names[..index].contains(name) {
                    return Err(fail(
                        ErrorKind::InvalidRequest,
                        "invalid or duplicate nominal selector",
                    ));
                }
                let present = if is_record {
                    source.records.iter().any(|item| item.name == *name)
                } else {
                    source.enums.iter().any(|item| item.name == *name)
                };
                if !present {
                    return Err(fail(ErrorKind::UnknownTarget, "unknown nominal selector"));
                }
            }
        }
        for (index, target) in callables.iter().enumerate() {
            cancel.check()?;
            locate(source, target)?;
            if callables[..index].contains(target) {
                return Err(fail(
                    ErrorKind::InvalidRequest,
                    "duplicate callable selector",
                ));
            }
        }
        cancel.check()
    }

    fn record(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Explicit { records, .. } => records.contains(&name),
        }
    }

    fn enumeration(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Explicit { enums, .. } => enums.contains(&name),
        }
    }

    fn callable(&self, function: &Function) -> bool {
        match self {
            Self::All => true,
            Self::Explicit { callables, .. } => callables
                .iter()
                .any(|target| target.name == function.name && target.receiver == function.receiver),
        }
    }
}

fn write_type(text: &mut BoundedText, kind: &Type) -> fmt::Result {
    match kind {
        Type::Unit => text.write_str("unit"),
        Type::Bool => text.write_str("bool"),
        Type::I64 => text.write_str("int"),
        Type::U64 => text.write_str("uint"),
        Type::F32 => text.write_str("float"),
        Type::Record(name) => write!(text, "record {name}"),
        Type::Enum(name) => write!(text, "enum {name}"),
        Type::List(element) => {
            text.write_str("list ")?;
            write_type(text, element)
        }
    }
}

fn append_bindings(text: &mut BoundedText, bindings: &[Binding]) -> fmt::Result {
    for binding in bindings {
        write!(text, " {} ", binding.name)?;
        write_type(text, &binding.ty)?;
    }
    Ok(())
}

struct Hex<'a>(&'a [u8; 32]);
impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn header(
    binding: &DraftBinding,
    target: &BodyTarget,
    index: usize,
    expected: &Type,
    limit: usize,
) -> Result<BoundedText> {
    let mut text = BoundedText::new(limit);
    let render = |text: &mut BoundedText| -> fmt::Result {
        write!(
            text,
            "ah1 analysis_only\nprofile {}\ncompiler_contract {}\ncompiler {}\nsource {}\nbase_semantic {}\nnode_index {}\nsketch {}\ndraft {}\nbuffer {} generation {}\ntarget ",
            binding.profile,
            binding.compiler_contract,
            Hex(&binding.compiler_sha256),
            Hex(&binding.source_sha256),
            Hex(&binding.base_semantic_sha256),
            Hex(&binding.node_index_sha256),
            Hex(&binding.sketch_sha256),
            Hex(&binding.draft_sha256),
            binding.buffer_version,
            binding.generation,
        )?;
        match &target.receiver {
            Some(receiver) => write!(text, "method {receiver} {}", target.name)?,
            None => write!(text, "function {}", target.name)?,
        }
        write!(text, " node {index}\nexpected ")?;
        write_type(text, expected)?;
        text.write_str("\neffects unproven\nresources unproven\ncapabilities unproven\nrequires whole_module_check invocation_limits\n")
    };
    render(&mut text).map_err(|_| {
        fail(
            if text.allocation_failed {
                ErrorKind::ResourceLimit
            } else {
                ErrorKind::OutputLimit
            },
            "query header allocation or byte limit refused",
        )
    })?;
    Ok(text)
}

fn compact_header(
    binding: &DraftBinding,
    target: &BodyTarget,
    expected: &Type,
    limit: usize,
) -> Result<BoundedText> {
    let mut text = BoundedText::new(limit);
    let render = |text: &mut BoundedText| -> fmt::Result {
        write!(
            text,
            "ac1 analysis_only\nbinding host_ticket\ndraft {}\ntarget ",
            Hex(&binding.draft_sha256),
        )?;
        match &target.receiver {
            Some(receiver) => write!(text, "method {receiver} {}", target.name)?,
            None => write!(text, "function {}", target.name)?,
        }
        text.write_str("\nexpected ")?;
        write_type(text, expected)?;
        text.write_str("\neffects unproven\nresources unproven\ncapabilities unproven\nrequires whole_module_check invocation_limits\n")
    };
    render(&mut text).map_err(|_| {
        fail(
            if text.allocation_failed {
                ErrorKind::ResourceLimit
            } else {
                ErrorKind::OutputLimit
            },
            "query header allocation or byte limit refused",
        )
    })?;
    Ok(text)
}

// All formatting writes are checked before reserving/appending. In particular,
// a large signature never allocates its full text merely to discover truncation.
struct BoundedText {
    text: String,
    limit: usize,
    allocation_failed: bool,
}
impl BoundedText {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            allocation_failed: false,
        }
    }
}
impl fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.len() > self.limit - self.text.len() {
            return Err(fmt::Error);
        }
        let needed = self.text.len() + value.len();
        if needed > self.text.capacity()
            && self
                .text
                .try_reserve_exact(needed - self.text.len())
                .is_err()
        {
            self.allocation_failed = true;
            return Err(fmt::Error);
        }
        self.text.push_str(value);
        Ok(())
    }
}

struct ContextBuilder<'a> {
    packet: BoundedText,
    visible: usize,
    total: usize,
    truncated: bool,
    max_items: usize,
    cancel: &'a Cancellation,
}
impl ContextBuilder<'_> {
    fn item(&mut self, render: impl FnOnce(&mut BoundedText) -> fmt::Result) -> Result<()> {
        self.cancel.check()?;
        self.total += 1;
        if self.truncated || self.visible == self.max_items {
            self.truncated = true;
            return Ok(());
        }
        let mut line = BoundedText::new(self.packet.limit - self.packet.text.len());
        if render(&mut line).is_err() {
            if line.allocation_failed {
                return Err(fail(
                    ErrorKind::ResourceLimit,
                    "query item allocation refused",
                ));
            }
            self.truncated = true;
            return Ok(());
        }
        self.cancel.check()?;
        // The line was bounded by exactly the remaining packet allowance.
        self.packet
            .write_str(&line.text)
            .map_err(|_| fail(ErrorKind::ResourceLimit, "query allocation refused"))?;
        self.visible += 1;
        Ok(())
    }
}

#[cfg(test)]
mod bounded_context_tests {
    use super::*;

    #[test]
    fn item_caps_and_truncation_never_run_later_formatters() {
        let cancel = Cancellation::default();
        let mut context = ContextBuilder {
            packet: BoundedText::new(4),
            visible: 0,
            total: 0,
            truncated: false,
            max_items: 0,
            cancel: &cancel,
        };
        context
            .item(|_| panic!("zero-item context must not format a signature"))
            .unwrap();
        assert_eq!(
            (context.visible, context.total, context.truncated),
            (0, 1, true)
        );
        let mut context = ContextBuilder {
            packet: BoundedText::new(4),
            visible: 0,
            total: 0,
            truncated: false,
            max_items: 100,
            cancel: &cancel,
        };
        context.item(|line| line.write_str("oversized")).unwrap();
        context
            .item(|_| panic!("truncated context must not format later signatures"))
            .unwrap();
        assert_eq!(
            (context.visible, context.total, context.truncated),
            (0, 2, true)
        );
        assert!(context.packet.text.is_empty());
    }

    #[test]
    fn oversized_fragment_never_appends_or_reserves_its_full_size() {
        let mut text = BoundedText::new(5);
        text.write_str("abc").unwrap();
        let capacity = text.text.capacity();
        assert!(text.write_str("123456789").is_err());
        assert_eq!(text.text, "abc");
        assert_eq!(text.text.capacity(), capacity);
    }

    #[test]
    fn shared_token_cancellation_during_render_refuses_publication() {
        use std::sync::mpsc;
        use std::time::Duration;
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let (start, begun) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            begun.recv_timeout(Duration::from_secs(2)).unwrap();
            worker_cancel.cancel();
            finished.send(()).unwrap();
        });
        let mut context = ContextBuilder {
            packet: BoundedText::new(4),
            visible: 0,
            total: 0,
            truncated: false,
            max_items: 1,
            cancel: &cancel,
        };
        let result = context.item(|line| {
            start.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(2)).unwrap();
            line.write_str("ok")
        });
        worker.join().unwrap();
        assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
        assert!(context.packet.text.is_empty());
        assert_eq!(context.visible, 0);
    }
}
