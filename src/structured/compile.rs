use super::*;
use crate::rules::Binary as RuleBinary;
use std::collections::BTreeSet;

const MAX_DECLARATIONS: usize = 512;
const MAX_BINDINGS: usize = 256;
const MAX_CODE: usize = 32_768;
// Compact source has its own byte bound. The FSM payload expands its operations
// into JSON objects and can use more bytes while preserving the released format.
const MAX_CANONICAL_SOURCE: usize = 1024 * 1024;

fn bound_canonical(length: usize) -> Result<(), Error> {
    if length > MAX_CANONICAL_SOURCE {
        return Err(error("structured program exceeds canonical byte budget"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) enum Operand {
    Const(Value),
    Local(usize),
    Input(usize),
    LocalField(usize, usize),
}

impl Operand {
    pub(super) fn cost(&self) -> u64 {
        if matches!(self, Self::LocalField(..)) {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Op {
    Operand(Operand),
    Store(usize),
    StoreField(usize, usize),
    Binary(Binary),
    BinaryOperands(Binary, Operand, Operand),
    RawRuleBinary(RuleBinary, Operand, Operand),
    Unary(Unary),
    Record(usize),
    Field(usize),
    Variant(usize, bool),
    IsVariant(usize),
    Payload(usize),
    List(usize),
    Empty,
    // These consume the following Empty/Last instruction as a second slot.
    // Keeping both slots preserves branch indices and all compile/cache limits.
    EmptyLocal(usize),
    Last,
    LastLocal(usize),
    Index,
    /// Consumes a checked list<f32> and produces u64. After the instruction's
    /// base fuel, debit one unit per element before evaluating the minimum.
    ArgMinF32,
    Push(usize),
    Pop(usize),
    Call(usize),
    HostCall {
        name: String,
        signature: HostSignature,
    },
    Jump(usize),
    JumpFalse(usize),
    JumpTrue(usize),
    Return,
}

#[derive(Clone, Debug)]
pub(crate) struct Instruction {
    pub(super) op: Op,
    pub(super) fuel: u64,
}

impl From<Op> for Instruction {
    fn from(op: Op) -> Self {
        let fuel = match &op {
            Op::Operand(operand) => operand.cost(),
            Op::BinaryOperands(_, left, right) | Op::RawRuleBinary(_, left, right) => {
                left.cost() + right.cost() + 1
            }
            _ => 1,
        };
        Self { op, fuel }
    }
}

#[derive(Debug)]
pub(crate) struct Code {
    pub(super) params: Vec<Type>,
    pub(super) result: Type,
    pub(super) slots: usize,
    pub(super) slot_types: Vec<Type>,
    pub(super) ops: Vec<Instruction>,
    #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
    pub(super) native: std::sync::OnceLock<Option<super::native_scalar::Scalar>>,
}

#[derive(Clone, Debug)]
pub struct Module {
    version: u16,
    canonical: Arc<[u8]>,
    pub(super) interface: Interface,
    pub(super) records: BTreeMap<String, Record>,
    pub(super) enums: BTreeMap<String, Enum>,
    pub(super) exports: BTreeMap<String, usize>,
    pub(super) code: Vec<Arc<Code>>,
    #[cfg(all(
        feature = "typed-jit",
        target_arch = "x86_64",
        any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
    ))]
    pub(super) native: Arc<[super::typed_jit::Cache]>,
}

// Preserve the released FSM payload without exposing a source format.
#[derive(Deserialize, Serialize)]
#[serde(remote = "Source", deny_unknown_fields)]
struct FsmSource {
    version: u16,
    #[serde(default)]
    inputs: Vec<Binding>,
    #[serde(default)]
    records: Vec<Record>,
    #[serde(default)]
    enums: Vec<Enum>,
    functions: Vec<Function>,
}

pub(super) fn encode_source(source: &Source) -> Result<Vec<u8>, Error> {
    let mut bytes = CanonicalWriter(Vec::new());
    FsmSource::serialize(source, &mut serde_json::Serializer::new(&mut bytes))
        .map_err(|e| error(e.to_string()))?;
    Ok(bytes.0)
}

// Reject overflow before extending the canonical output buffer. Admitted
// encodings are unchanged; an oversized public Source never gets a complete
// oversized canonical copy merely so its length can be checked afterward.
struct CanonicalWriter(Vec<u8>);

impl std::io::Write for CanonicalWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_CANONICAL_SOURCE - self.0.len() {
            return Err(std::io::Error::other(
                "structured program exceeds canonical byte budget",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod canonical_writer_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn canonical_overflow_never_appends_bytes_past_the_limit() {
        let mut writer = CanonicalWriter(Vec::new());
        writer.write_all(&vec![b'x'; MAX_CANONICAL_SOURCE]).unwrap();
        assert_eq!(writer.0.len(), MAX_CANONICAL_SOURCE);
        assert!(writer.write_all(b"x").is_err());
        assert_eq!(writer.0.len(), MAX_CANONICAL_SOURCE);
        let mut fresh = CanonicalWriter(Vec::new());
        assert!(
            fresh
                .write_all(&vec![b'x'; MAX_CANONICAL_SOURCE + 1])
                .is_err()
        );
        assert!(fresh.0.is_empty());
    }
}

fn decode_source(source: &[u8]) -> Result<Source, Error> {
    bound_canonical(source.len())?;
    let text = std::str::from_utf8(source).map_err(|e| error(e.to_string()))?;
    let mut parser = serde_json::Deserializer::from_str(text);
    let source = FsmSource::deserialize(&mut parser).map_err(|e| error(e.to_string()))?;
    parser.end().map_err(|e| error(e.to_string()))?;
    Ok(source)
}

impl Module {
    /// Read a released JSON module for migration to compact source or FXH.
    pub fn compile_legacy_json(source: &str) -> Result<Self, Error> {
        Self::compile(decode_source(source.as_bytes())?)
    }

    pub fn compile_compact(text: &str) -> Result<Self, Error> {
        Self::compile(Source::parse_compact(text)?)
    }

    pub fn compile(source: Source) -> Result<Self, Error> {
        Self::compile_borrowed(&source)
    }

    pub(crate) fn compile_borrowed(source: &Source) -> Result<Self, Error> {
        Self::compile_source(source, false, false)
    }

    /// A unified document may contain only native items. Its checked module is
    /// then empty, with no synthetic or hidden exports.
    pub(crate) fn compile_unified(source: &Source) -> Result<Self, Error> {
        Self::compile_source(source, false, true)
    }

    /// Only the opaque checked rule projection can hold this module. Its
    /// compiler input permits rule-only operations and has no released encoding.
    pub(crate) fn compile_rule_projection(source: &Source) -> Result<Self, Error> {
        Self::compile_source(source, true, false)
    }

    fn compile_source(
        source: &Source,
        rule_projection: bool,
        allow_empty: bool,
    ) -> Result<Self, Error> {
        if !(1..=VERSION).contains(&source.version) {
            return Err(error(format!(
                "unsupported module version {}",
                source.version
            )));
        }
        let mut env = Environment::with_empty(source, allow_empty)?;
        env.rule_projection = rule_projection;
        let mut code = Vec::with_capacity(source.functions.len());
        let mut exports = BTreeMap::new();
        let mut signatures = BTreeMap::new();
        let mut operations = 0;
        for (index, function) in source.functions.iter().enumerate() {
            let body = compile_body(&env, function, &function.body)?;
            operations += body.ops.len();
            if operations > MAX_CODE {
                return Err(error("module instruction count exceeds limit"));
            }
            if function.export {
                exports.insert(function.name.clone(), index);
                signatures.insert(
                    function.name.clone(),
                    (body.params.clone(), body.result.clone()),
                );
            }
            code.push(Arc::new(body));
        }
        let interface = env.interface(&source.inputs, signatures);
        let canonical = if rule_projection {
            Arc::from([])
        } else {
            encode_source(source)?.into()
        };
        #[cfg(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        ))]
        let native = (0..code.len()).map(|_| Default::default()).collect();
        let module = Self {
            version: source.version,
            canonical,
            interface,
            records: env.records,
            enums: env.enums,
            exports,
            code,
            #[cfg(all(
                feature = "typed-jit",
                target_arch = "x86_64",
                any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
            ))]
            native,
        };
        module.host_requirements()?;
        Ok(module)
    }

    // The collaboration path only changes bodies. The environment's function
    // indices, parameter slots, nominal schemas and export table stay fixed.
    pub(crate) fn replace_bodies(
        &self,
        source: &Source,
        checker: &BodyChecker<'_>,
        changed: &BTreeSet<usize>,
    ) -> Result<Self, Error> {
        let mut module = self.clone();
        // Code bodies can be reused across generations. Inlined native calls
        // depend on the complete generation, including unchanged callers.
        #[cfg(all(
            feature = "typed-jit",
            target_arch = "x86_64",
            any(target_os = "windows", all(target_os = "linux", target_env = "gnu"))
        ))]
        if !changed.is_empty() {
            module.native = (0..self.code.len()).map(|_| Default::default()).collect();
        }
        for &index in changed {
            let function = &source.functions[index];
            module.code[index] = checker.compiled_body(index, &function.body)?;
        }
        if module.code.iter().map(|body| body.ops.len()).sum::<usize>() > MAX_CODE {
            return Err(error("module instruction count exceeds limit"));
        }
        module.canonical = encode_source(source)?.into();
        module.host_requirements()?;
        Ok(module)
    }

    pub(crate) fn canonical_source(&self) -> &[u8] {
        &self.canonical
    }

    /// Resolved function dependencies, including calls through typed methods.
    /// Use the compiler's indices rather than repeating source name resolution
    /// in collaboration tooling. Indices refer to the admitted source order.
    pub(crate) fn call_dependencies(&self) -> Vec<BTreeSet<usize>> {
        (0..self.code.len())
            .map(|index| self.call_dependencies_at(index))
            .collect()
    }

    /// Read only the compiled body being replaced; unchanged bodies already
    /// have immutable dependency entries in the authoring snapshot.
    pub(crate) fn call_dependencies_at(&self, index: usize) -> BTreeSet<usize> {
        self.code[index]
            .ops
            .iter()
            .filter_map(|instruction| match instruction.op {
                Op::Call(index) => Some(index),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn checked_source(&self) -> Result<Source, Error> {
        decode_source(self.canonical_source())
    }

    /// Portable envelope of the validated canonical source. Decoding rechecks
    /// the module before it can be executed.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAGIC.len() + 2 + self.canonical.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.canonical);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let (version, source) = bytes
            .strip_prefix(MAGIC)
            .and_then(|bytes| bytes.split_first_chunk::<2>())
            .ok_or_else(|| error("unknown structured module envelope"))?;
        let decoded = decode_source(source)?;
        let module = Self::compile_unified(&decoded)?;
        if module.version != u16::from_le_bytes(*version) {
            return Err(error(
                "structured module envelope version differs from source",
            ));
        }
        Ok(module)
    }
}

pub(crate) struct BodyChecker<'a> {
    env: Environment<'a>,
    // Scoped to one preparation, so compiler/environment changes cannot reuse
    // old code. Borrow the checked AST rather than duplicating proposal bodies.
    checked: BTreeMap<usize, (&'a [Stmt], Arc<Code>)>,
    cached_instructions: usize,
}

impl<'a> BodyChecker<'a> {
    pub(crate) fn new(source: &'a Source) -> Result<Self, Error> {
        Ok(Self {
            env: Environment::new(source)?,
            checked: BTreeMap::new(),
            cached_instructions: 0,
        })
    }

    pub(crate) fn check(&mut self, index: usize, body: &'a [Stmt]) -> Result<(), Error> {
        let compiled = self.compiled_body(index, body)?;
        let previous = self
            .checked
            .get(&index)
            .map_or(0, |(_, code)| code.ops.len());
        let retained = self.cached_instructions - previous + compiled.ops.len();
        // A hostile/conflicting batch must not retain MAX_EDITS full modules.
        // Cache at most one body's code per declaration and at most the admitted
        // module instruction budget. Uncached valid bodies compile on selection.
        if retained <= MAX_CODE {
            self.checked.insert(index, (body, compiled));
            self.cached_instructions = retained;
        }
        Ok(())
    }

    /// Check a projection argument against the same environment and expression
    /// rules as ordinary source, without changing the emitted program.
    pub(crate) fn expression_type(&self, index: usize, expr: &Expr) -> Result<Type, Error> {
        let function = self
            .env
            .functions
            .get(index)
            .ok_or_else(|| error("projection function index out of range"))?;
        Compiler::new(&self.env, function)?.expr(expr, 0)
    }

    fn compiled_body(&self, index: usize, body: &[Stmt]) -> Result<Arc<Code>, Error> {
        if let Some((checked, compiled)) = self.checked.get(&index)
            && *checked == body
        {
            return Ok(Arc::clone(compiled));
        }
        compile_body(&self.env, &self.env.functions[index], body).map(Arc::new)
    }
}

fn compile_body(env: &Environment<'_>, function: &Function, body: &[Stmt]) -> Result<Code, Error> {
    let mut compiler = Compiler::new(env, function)?;
    if !compiler.block(body, 0)? {
        return Err(error(format!(
            "{} can finish without returning",
            function.name
        )));
    }
    if compiler.ops.len() > MAX_CODE {
        return Err(error("replacement instruction count exceeds limit"));
    }
    let mut slot_types = vec![Type::Unit; compiler.slots.len()];
    for (index, ty) in compiler.slots.into_values() {
        slot_types[index] = ty;
    }
    Ok(Code {
        params: function.params.iter().map(|p| p.ty.clone()).collect(),
        result: function.result.clone(),
        slots: slot_types.len(),
        slot_types,
        ops: compiler.ops.into_iter().map(Instruction::from).collect(),
        #[cfg(all(feature = "native-jit", target_os = "windows", target_arch = "x86_64"))]
        native: std::sync::OnceLock::new(),
    })
}

struct Environment<'a> {
    version: u16,
    rule_projection: bool,
    functions: &'a [Function],
    function_ids: BTreeMap<(Option<String>, String), usize>,
    inputs: BTreeMap<String, (usize, Type)>,
    records: BTreeMap<String, Record>,
    enums: BTreeMap<String, Enum>,
}

impl<'a> Environment<'a> {
    fn new(source: &'a Source) -> Result<Self, Error> {
        Self::with_empty(source, false)
    }
    fn with_empty(source: &'a Source, allow_empty: bool) -> Result<Self, Error> {
        if (!allow_empty && source.functions.is_empty())
            || source.functions.len() > MAX_DECLARATIONS
            || source.records.len() + source.enums.len() > MAX_DECLARATIONS
        {
            return Err(error("module declaration count outside limits"));
        }
        let mut env = Self {
            version: source.version,
            rule_projection: false,
            functions: &source.functions,
            function_ids: BTreeMap::new(),
            inputs: bindings(source.inputs.iter())?,
            records: BTreeMap::new(),
            enums: BTreeMap::new(),
        };
        for record in &source.records {
            identifier(&record.name)?;
            bindings(record.fields.iter())?;
            if env
                .records
                .insert(record.name.clone(), record.clone())
                .is_some()
            {
                return Err(error(format!("duplicate record {}", record.name)));
            }
        }
        for enumeration in &source.enums {
            identifier(&enumeration.name)?;
            if env.records.contains_key(&enumeration.name)
                || env
                    .enums
                    .insert(enumeration.name.clone(), enumeration.clone())
                    .is_some()
            {
                return Err(error(format!("duplicate type {}", enumeration.name)));
            }
            if !(1..=MAX_BINDINGS).contains(&enumeration.variants.len()) {
                return Err(error("enum variant count outside limits"));
            }
            let mut names = BTreeSet::new();
            for variant in &enumeration.variants {
                identifier(&variant.name)?;
                if !names.insert(&variant.name) {
                    return Err(error(format!("duplicate variant {}", variant.name)));
                }
            }
        }
        for ty in source
            .inputs
            .iter()
            .map(|input| &input.ty)
            .chain(
                source
                    .records
                    .iter()
                    .flat_map(|r| r.fields.iter().map(|f| &f.ty)),
            )
            .chain(
                source
                    .enums
                    .iter()
                    .flat_map(|e| e.variants.iter().filter_map(|v| v.payload.as_ref())),
            )
        {
            env.check_type(ty)?;
        }
        for (index, function) in source.functions.iter().enumerate() {
            identifier(&function.name)?;
            for ty in function
                .params
                .iter()
                .chain(&function.locals)
                .map(|b| &b.ty)
                .chain(std::iter::once(&function.result))
            {
                env.check_type(ty)?;
            }
            if let Some(receiver) = &function.receiver
                && (function.export
                    || function
                        .params
                        .first()
                        .is_none_or(|p| p.name != "self" || p.ty != Type::Record(receiver.clone())))
            {
                return Err(error(format!(
                    "method {} needs a record self parameter",
                    function.name
                )));
            }
            if env
                .function_ids
                .insert((function.receiver.clone(), function.name.clone()), index)
                .is_some()
            {
                return Err(error(format!(
                    "duplicate function or method {}",
                    function.name
                )));
            }
        }
        Ok(env)
    }

    fn check_type(&self, ty: &Type) -> Result<(), Error> {
        self.type_depth(ty, 0, &mut BTreeSet::new(), &mut BTreeMap::new())?;
        Ok(())
    }

    fn type_depth(
        &self,
        ty: &Type,
        level: usize,
        visiting: &mut BTreeSet<String>,
        known: &mut BTreeMap<String, usize>,
    ) -> Result<usize, Error> {
        if level >= MAX_DEPTH {
            return Err(error("type nesting limit exceeded"));
        }
        let name = match ty {
            Type::List(element) => {
                return Ok(1 + self.type_depth(element, level + 1, visiting, known)?);
            }
            Type::Record(name) | Type::Enum(name) => name,
            _ => return Ok(1),
        };
        if let Some(&depth) = known.get(name) {
            if level + depth > MAX_DEPTH {
                return Err(error("type nesting limit exceeded"));
            }
            return Ok(depth);
        }
        if !visiting.insert(name.clone()) {
            return Err(error(format!("recursive value type {name}")));
        }
        let children: Vec<_> = match ty {
            Type::Record(_) => self
                .records
                .get(name)
                .ok_or_else(|| error(format!("unknown record {name}")))?
                .fields
                .iter()
                .map(|f| &f.ty)
                .collect(),
            Type::Enum(_) => self
                .enums
                .get(name)
                .ok_or_else(|| error(format!("unknown enum {name}")))?
                .variants
                .iter()
                .filter_map(|v| v.payload.as_ref())
                .collect(),
            _ => unreachable!("named type"),
        };
        let mut depth = 1;
        for child in children {
            depth = depth.max(1 + self.type_depth(child, level + 1, visiting, known)?);
        }
        visiting.remove(name);
        known.insert(name.clone(), depth);
        Ok(depth)
    }

    fn interface(
        &self,
        inputs: &[Binding],
        exports: BTreeMap<String, (Vec<Type>, Type)>,
    ) -> Interface {
        let mut records = BTreeMap::new();
        let mut enums = BTreeMap::new();
        let mut types: Vec<_> = inputs
            .iter()
            .map(|b| &b.ty)
            .chain(
                exports
                    .values()
                    .flat_map(|(params, result)| params.iter().chain(std::iter::once(result))),
            )
            .collect();
        while let Some(ty) = types.pop() {
            match ty {
                Type::List(element) => types.push(element),
                Type::Record(name) if !records.contains_key(name) => {
                    let record = &self.records[name];
                    records.insert(name.clone(), record.clone());
                    types.extend(record.fields.iter().map(|f| &f.ty));
                }
                Type::Enum(name) if !enums.contains_key(name) => {
                    let enumeration = &self.enums[name];
                    enums.insert(name.clone(), enumeration.clone());
                    types.extend(
                        enumeration
                            .variants
                            .iter()
                            .filter_map(|v| v.payload.as_ref()),
                    );
                }
                _ => {}
            }
        }
        Interface {
            inputs: inputs.to_vec(),
            exports,
            records,
            enums,
        }
    }

    fn field(&self, ty: &Type, field: &str) -> Result<(usize, Type), Error> {
        let Type::Record(record) = ty else {
            return Err(error("field needs a record"));
        };
        self.records[record]
            .fields
            .iter()
            .enumerate()
            .find(|(_, binding)| binding.name == field)
            .map(|(index, binding)| (index, binding.ty.clone()))
            .ok_or_else(|| error(format!("unknown field {record}.{field}")))
    }

    fn variant(&self, name: &str, variant: &str) -> Result<(usize, Option<Type>), Error> {
        self.enums
            .get(name)
            .and_then(|definition| {
                definition
                    .variants
                    .iter()
                    .enumerate()
                    .find(|(_, v)| v.name == variant)
            })
            .map(|(tag, definition)| (tag, definition.payload.clone()))
            .ok_or_else(|| error(format!("unknown variant {name}.{variant}")))
    }
}

fn identifier(name: &str) -> Result<(), Error> {
    let mut bytes = name.bytes();
    if name.len() > 64
        || !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(error(format!("invalid identifier {name:?}")));
    }
    Ok(())
}

fn bindings<'a>(
    list: impl Iterator<Item = &'a Binding>,
) -> Result<BTreeMap<String, (usize, Type)>, Error> {
    let mut result = BTreeMap::new();
    for (index, binding) in list.enumerate() {
        if index >= MAX_BINDINGS {
            return Err(error("binding count exceeds limit"));
        }
        identifier(&binding.name)?;
        if result
            .insert(binding.name.clone(), (index, binding.ty.clone()))
            .is_some()
        {
            return Err(error(format!("duplicate binding {}", binding.name)));
        }
    }
    Ok(result)
}

fn require(actual: &Type, expected: &Type) -> Result<(), Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(error(format!("expected {expected:?}, got {actual:?}")))
    }
}

fn list_element(ty: Type) -> Result<Type, Error> {
    if let Type::List(element) = ty {
        Ok(*element)
    } else {
        Err(error("list operation needs a list"))
    }
}

struct Loop {
    start: usize,
    exits: Vec<usize>,
}

struct Compiler<'a, 'src> {
    env: &'a Environment<'src>,
    result: &'a Type,
    slots: BTreeMap<String, (usize, Type)>,
    ops: Vec<Op>,
    loops: Vec<Loop>,
}

impl<'a, 'src> Compiler<'a, 'src> {
    fn new(env: &'a Environment<'src>, function: &'a Function) -> Result<Self, Error> {
        Ok(Self {
            env,
            result: &function.result,
            slots: bindings(function.params.iter().chain(&function.locals))?,
            ops: Vec::new(),
            loops: Vec::new(),
        })
    }

    fn local(&self, name: &str) -> Result<(usize, Type), Error> {
        self.slots
            .get(name)
            .cloned()
            .ok_or_else(|| error(format!("unknown local {name}")))
    }

    fn block(&mut self, body: &[Stmt], depth: usize) -> Result<bool, Error> {
        if depth >= MAX_DEPTH {
            return Err(error("statement nesting limit exceeded"));
        }
        let mut terminal = false;
        for stmt in body {
            if terminal {
                return Err(error("unreachable statement"));
            }
            terminal = self.statement(stmt, depth + 1)?;
            if self.ops.len() > MAX_CODE {
                return Err(error("function instruction count exceeds limit"));
            }
        }
        Ok(terminal)
    }

    fn patch(&mut self, index: usize) {
        let target = self.ops.len();
        match &mut self.ops[index] {
            Op::Jump(at) | Op::JumpFalse(at) | Op::JumpTrue(at) => *at = target,
            _ => unreachable!("only branches can be patched"),
        }
    }

    fn statement(&mut self, stmt: &Stmt, depth: usize) -> Result<bool, Error> {
        let op = match stmt {
            Stmt::Set { local, value } => {
                let (index, ty) = self.local(local)?;
                require(&self.expr(value, depth)?, &ty)?;
                Op::Store(index)
            }
            Stmt::SetField {
                local,
                field,
                value,
            } => {
                let (index, ty) = self.local(local)?;
                let (field, ty) = self.env.field(&ty, field)?;
                require(&self.expr(value, depth)?, &ty)?;
                Op::StoreField(index, field)
            }
            Stmt::Push { local, value } => {
                let (index, ty) = self.local(local)?;
                require(&self.expr(value, depth)?, &list_element(ty)?)?;
                Op::Push(index)
            }
            Stmt::Pop { local } => {
                let (index, ty) = self.local(local)?;
                list_element(ty)?;
                Op::Pop(index)
            }
            Stmt::If {
                test,
                then,
                otherwise,
            } => {
                require(&self.expr(test, depth)?, &Type::Bool)?;
                let branch = self.ops.len();
                self.ops.push(Op::JumpFalse(0));
                let then_terminal = self.block(then, depth)?;
                let end = if then_terminal {
                    None
                } else {
                    let index = self.ops.len();
                    self.ops.push(Op::Jump(0));
                    Some(index)
                };
                self.patch(branch);
                let else_terminal = self.block(otherwise, depth)?;
                if let Some(index) = end {
                    self.patch(index);
                }
                return Ok(then_terminal && else_terminal);
            }
            Stmt::While { test, body } => {
                let start = self.ops.len();
                require(&self.expr(test, depth)?, &Type::Bool)?;
                let exit = self.ops.len();
                self.ops.push(Op::JumpFalse(0));
                self.loops.push(Loop {
                    start,
                    exits: vec![exit],
                });
                if !self.block(body, depth)? {
                    self.ops.push(Op::Jump(start));
                }
                for exit in self.loops.pop().expect("active loop").exits {
                    self.patch(exit);
                }
                return Ok(false);
            }
            Stmt::Break => {
                let active = self
                    .loops
                    .last_mut()
                    .ok_or_else(|| error("break outside loop"))?;
                active.exits.push(self.ops.len());
                self.ops.push(Op::Jump(0));
                return Ok(true);
            }
            Stmt::Continue => {
                let active = self
                    .loops
                    .last()
                    .ok_or_else(|| error("continue outside loop"))?;
                self.ops.push(Op::Jump(active.start));
                return Ok(true);
            }
            Stmt::Return { value } => {
                require(&self.expr(value, depth)?, self.result)?;
                self.ops.push(Op::Return);
                return Ok(true);
            }
        };
        self.ops.push(op);
        Ok(false)
    }

    fn expr(&mut self, expr: &Expr, depth: usize) -> Result<Type, Error> {
        if depth >= MAX_DEPTH {
            return Err(error("expression nesting limit exceeded"));
        }
        if self.ops.len() >= MAX_CODE {
            return Err(error("function instruction count exceeds limit"));
        }
        let next = depth + 1;
        let (op, ty) = match expr {
            Expr::Unit => (Op::Operand(Operand::Const(Value::Unit)), Type::Unit),
            Expr::Bool { value } => (Op::Operand(Operand::Const(Value::Bool(*value))), Type::Bool),
            Expr::I64 { value } => (Op::Operand(Operand::Const(Value::I64(*value))), Type::I64),
            Expr::U64 { value } => (Op::Operand(Operand::Const(Value::U64(*value))), Type::U64),
            Expr::F32 { bits } => (Op::Operand(Operand::Const(Value::F32(*bits))), Type::F32),
            Expr::Local { name } => {
                let (index, ty) = self.local(name)?;
                (Op::Operand(Operand::Local(index)), ty)
            }
            Expr::Input { name } => {
                let (index, ty) = self
                    .env
                    .inputs
                    .get(name)
                    .ok_or_else(|| error(format!("unknown input {name}")))?;
                (Op::Operand(Operand::Input(*index)), ty.clone())
            }
            Expr::Binary { kind, left, right } => return self.binary(*kind, left, right, next),
            Expr::Unary { kind, value } => {
                let (argument, result) = match kind {
                    Unary::Not => (Type::Bool, Type::Bool),
                    Unary::AbsF32 => (Type::F32, Type::F32),
                    Unary::U64ToF32 => (Type::U64, Type::F32),
                    Unary::I64ToF32 => (Type::I64, Type::F32),
                    Unary::F32ToI64 => (Type::F32, Type::I64),
                    Unary::SqrtF32 => (Type::F32, Type::F32),
                };
                require(&self.expr(value, next)?, &argument)?;
                (Op::Unary(*kind), result)
            }
            Expr::Record { name, fields } => {
                let record = self
                    .env
                    .records
                    .get(name)
                    .ok_or_else(|| error(format!("unknown record {name}")))?;
                self.arguments(fields, &record.fields, next)?;
                (Op::Record(fields.len()), Type::Record(name.clone()))
            }
            Expr::Field { record, name } => {
                if let Expr::Local { name: local } = record.as_ref() {
                    let (slot, ty) = self.local(local)?;
                    let (index, ty) = self.env.field(&ty, name)?;
                    self.ops.push(Op::Operand(Operand::LocalField(slot, index)));
                    return Ok(ty);
                }
                let record = self.expr(record, next)?;
                let (index, ty) = self.env.field(&record, name)?;
                (Op::Field(index), ty)
            }
            Expr::Variant {
                name,
                variant,
                payload,
            } => {
                let (tag, ty) = self.env.variant(name, variant)?;
                match (ty, payload) {
                    (Some(ty), Some(payload)) => require(&self.expr(payload, next)?, &ty)?,
                    (None, None) => {}
                    _ => return Err(error(format!("wrong payload for {name}.{variant}"))),
                }
                (
                    Op::Variant(tag, payload.is_some()),
                    Type::Enum(name.clone()),
                )
            }
            Expr::IsVariant { value, variant } | Expr::Payload { value, variant } => {
                let Type::Enum(name) = self.expr(value, next)? else {
                    return Err(error("variant operation needs an enum"));
                };
                let (tag, payload) = self.env.variant(&name, variant)?;
                if matches!(expr, Expr::IsVariant { .. }) {
                    (Op::IsVariant(tag), Type::Bool)
                } else {
                    let ty =
                        payload.ok_or_else(|| error(format!("{name}.{variant} has no payload")))?;
                    (Op::Payload(tag), ty)
                }
            }
            Expr::List { element, items } => {
                self.env.check_type(element)?;
                for item in items {
                    require(&self.expr(item, next)?, element)?;
                }
                (Op::List(items.len()), Type::List(Box::new(element.clone())))
            }
            Expr::Empty { list } => {
                let first = self.ops.len();
                list_element(self.expr(list, next)?)?;
                if let [Op::Operand(Operand::Local(index))] = &self.ops[first..] {
                    self.ops[first] = Op::EmptyLocal(*index);
                }
                (Op::Empty, Type::Bool)
            }
            Expr::Last { list } => {
                let first = self.ops.len();
                let element = list_element(self.expr(list, next)?)?;
                if let [Op::Operand(Operand::Local(index))] = &self.ops[first..] {
                    self.ops[first] = Op::LastLocal(*index);
                }
                (Op::Last, element)
            }
            Expr::Index { list, index } => {
                let ty = list_element(self.expr(list, next)?)?;
                require(&self.expr(index, next)?, &Type::U64)?;
                (Op::Index, ty)
            }
            Expr::ArgMinF32 { list } => {
                if self.env.version < 2 {
                    return Err(error("arg_min_f32 requires structured module version 2"));
                }
                require(&list_element(self.expr(list, next)?)?, &Type::F32)?;
                (Op::ArgMinF32, Type::U64)
            }
            Expr::Call { name, args } if self.env.rule_projection => {
                // Rule lowering represents raw operators as private calls on
                // the two signed locals that hold the rule stack operands.
                let operation = RuleBinary::raw_f32_token(name)
                    .ok_or_else(|| error("unknown raw rule operator"))?;
                let [Expr::Local { name: left }, Expr::Local { name: right }] = args.as_slice()
                else {
                    return Err(error("raw rule operator needs two locals"));
                };
                let (left, left_ty) = self.local(left)?;
                let (right, right_ty) = self.local(right)?;
                require(&left_ty, &Type::I64)?;
                require(&right_ty, &Type::I64)?;
                (
                    Op::RawRuleBinary(operation, Operand::Local(left), Operand::Local(right)),
                    Type::I64,
                )
            }
            Expr::Call { name, args } => self.call(None, name, args, next)?,
            Expr::HostCall {
                name,
                params,
                result,
                args,
            } => {
                if self.env.rule_projection {
                    return Err(error(
                        "native host services are not rule projection operators",
                    ));
                }
                let valid = !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().enumerate().all(|(i, b)| {
                        b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit())
                    });
                if !valid || params.len() > MAX_BINDINGS || params.len() != args.len() {
                    return Err(error("invalid host service name or argument count"));
                }
                self.env.check_type(result)?;
                for (arg, ty) in args.iter().zip(params) {
                    self.env.check_type(ty)?;
                    require(&self.expr(arg, next)?, ty)?;
                }
                (
                    Op::HostCall {
                        name: name.clone(),
                        signature: HostSignature {
                            params: params.clone(),
                            result: result.clone(),
                        },
                    },
                    result.clone(),
                )
            }
            Expr::Method {
                receiver,
                name,
                args,
            } => {
                let Type::Record(receiver) = self.expr(receiver, next)? else {
                    return Err(error("method receiver needs a record"));
                };
                self.call(Some(receiver), name, args, next)?
            }
        };
        self.ops.push(op);
        Ok(ty)
    }

    fn binary(
        &mut self,
        kind: Binary,
        left: &Expr,
        right: &Expr,
        depth: usize,
    ) -> Result<Type, Error> {
        use Binary::*;
        let first = self.ops.len();
        let ty = self.expr(left, depth)?;
        let signed_and = self.env.rule_projection && kind == And && ty == Type::I64;
        if matches!(kind, And | Or) && !signed_and {
            require(&ty, &Type::Bool)?;
            let branch = self.ops.len();
            self.ops.push(if matches!(kind, And) {
                Op::JumpFalse(0)
            } else {
                Op::JumpTrue(0)
            });
            require(&self.expr(right, depth)?, &Type::Bool)?;
            let end = self.ops.len();
            self.ops.push(Op::Jump(0));
            self.patch(branch);
            self.ops
                .push(Op::Operand(Operand::Const(Value::Bool(matches!(kind, Or)))));
            self.patch(end);
            return Ok(Type::Bool);
        }
        require(&self.expr(right, depth)?, &ty)?;
        let numeric = matches!(ty, Type::I64 | Type::U64 | Type::F32);
        let result = match kind {
            Eq if numeric || ty == Type::Bool => Type::Bool,
            Lt | Le if numeric => Type::Bool,
            Add | Sub | Mul | Div if numeric => ty,
            Rem if matches!(ty, Type::I64 | Type::U64) => ty,
            And if signed_and => ty,
            _ => return Err(error(format!("invalid {kind:?} operands of type {ty:?}"))),
        };
        if let [Op::Operand(left), Op::Operand(right)] = &self.ops[first..] {
            let combined = Op::BinaryOperands(kind, left.clone(), right.clone());
            self.ops.truncate(first);
            self.ops.push(combined);
        } else {
            self.ops.push(Op::Binary(kind));
        }
        Ok(result)
    }

    fn call(
        &mut self,
        receiver: Option<String>,
        name: &str,
        args: &[Expr],
        depth: usize,
    ) -> Result<(Op, Type), Error> {
        let offset = usize::from(receiver.is_some());
        let index = *self
            .env
            .function_ids
            .get(&(receiver, name.to_owned()))
            .ok_or_else(|| error(format!("unknown function or method {name}")))?;
        let function = &self.env.functions[index];
        self.arguments(args, &function.params[offset..], depth)?;
        Ok((Op::Call(index), function.result.clone()))
    }

    fn arguments(&mut self, args: &[Expr], params: &[Binding], depth: usize) -> Result<(), Error> {
        if args.len() != params.len() {
            return Err(error("incorrect number of arguments"));
        }
        for (arg, param) in args.iter().zip(params) {
            require(&self.expr(arg, depth)?, &param.ty)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod prepared_body_tests {
    use super::*;

    #[test]
    fn checked_code_cache_is_bounded_and_uncached_selection_keeps_module_limits() {
        let mut text = String::from("v1 ");
        for index in 0..128 {
            text.push_str(&format!("(pub f{index} () i64 (x:i64) (ret 0)) "));
        }
        let source = Source::parse_compact(&text).unwrap();
        let original = Module::compile_borrowed(&source).unwrap();
        let mut body = vec![
            Stmt::Set {
                local: "x".into(),
                value: Expr::I64 { value: 17 },
            };
            255
        ];
        body.push(Stmt::Return {
            value: Expr::Local { name: "x".into() },
        });
        let mut checker = BodyChecker::new(&source).unwrap();
        for index in 0..128 {
            checker.check(index, &body).unwrap();
        }
        assert!(checker.cached_instructions <= MAX_CODE);
        assert!(!checker.checked.contains_key(&127));
        // The last proposal did not fit in the compile cache. It is still
        // recompiled and admitted correctly when selected by composition.
        let mut edited = source.clone();
        edited.functions[127].body = body.clone();
        let next = original
            .replace_bodies(&edited, &checker, &BTreeSet::from([127]))
            .unwrap();
        let cold = Module::compile(edited.clone()).unwrap();
        assert_eq!(next.encode(), cold.encode());
        assert_eq!(
            next.run(Invocation {
                export: "f127",
                inputs: &[],
                arguments: &[],
                limits: Limits::default(),
            })
            .unwrap(),
            Value::I64(17)
        );
        // Individual body checks do not waive the composed module's limits.
        for function in &mut edited.functions {
            function.body = body.clone();
        }
        let error = original
            .replace_bodies(&edited, &checker, &(0..128).collect())
            .unwrap_err();
        assert!(error.to_string().contains("module instruction count"));
    }

    #[test]
    fn checked_code_preserves_signed_zero_and_nan_payload_bits() {
        let source = Source::parse_compact("v1 (pub value () f32 () (ret 0f))").unwrap();
        let original = Module::compile_borrowed(&source).unwrap();
        let patterns = [0, 0x8000_0000, 0x7fc0_0001, 0x7fc0_0002, 0x7f80_0001];
        let bodies: Vec<_> = patterns
            .iter()
            .map(|&bits| {
                vec![Stmt::Return {
                    value: Expr::F32 { bits },
                }]
            })
            .collect();
        let mut checker = BodyChecker::new(&source).unwrap();
        for body in &bodies {
            checker.check(0, body).unwrap();
        }
        for (&bits, body) in patterns.iter().zip(&bodies).rev() {
            let mut edited = source.clone();
            edited.functions[0].body = body.clone();
            let next = original
                .replace_bodies(&edited, &checker, &BTreeSet::from([0]))
                .unwrap();
            assert_eq!(
                next.run(Invocation {
                    export: "value",
                    inputs: &[],
                    arguments: &[],
                    limits: Limits::default(),
                })
                .unwrap(),
                Value::F32(bits)
            );
            assert_eq!(next.encode(), Module::compile(edited).unwrap().encode());
        }
    }

    #[test]
    fn code_reuse_is_bound_to_the_exact_checked_body_and_declaration() {
        let source = Source::parse_compact(
            "v1 (pub signed () i64 () (ret 0)) (pub unsigned () u64 () (ret 0u))",
        )
        .unwrap();
        let original = Module::compile_borrowed(&source).unwrap();
        let first = vec![Stmt::Return {
            value: Expr::I64 { value: 5 },
        }];
        let second = vec![Stmt::Return {
            value: Expr::I64 { value: 9 },
        }];
        let mut checker = BodyChecker::new(&source).unwrap();
        checker.check(0, &first).unwrap();
        assert!(checker.check(1, &first).is_err());
        checker.check(0, &second).unwrap();
        let mut edited = source.clone();
        edited.functions[0].body = first.clone();
        let next = original
            .replace_bodies(&edited, &checker, &BTreeSet::from([0]))
            .unwrap();
        assert_eq!(
            next.run(Invocation {
                export: "signed",
                inputs: &[],
                arguments: &[],
                limits: Limits::default(),
            })
            .unwrap(),
            Value::I64(5)
        );
    }
}
