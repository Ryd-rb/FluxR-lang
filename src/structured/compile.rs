use super::*;
use std::collections::BTreeSet;

const MAX_DECLARATIONS: usize = 256;
const MAX_BINDINGS: usize = 256;
const MAX_CODE: usize = 32_768;

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
    Unary(Unary),
    Record(usize),
    Field(usize),
    Variant(usize, bool),
    IsVariant(usize),
    Payload(usize),
    List(usize),
    Empty,
    Last,
    Index,
    /// Consumes a checked list<f32> and produces u64. After the instruction's
    /// base fuel, debit one unit per element before evaluating the minimum.
    ArgMinF32,
    Push(usize),
    Pop(usize),
    Call(usize),
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
            Op::BinaryOperands(_, left, right) => left.cost() + right.cost() + 1,
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
    #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
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
    let mut bytes = Vec::new();
    FsmSource::serialize(source, &mut serde_json::Serializer::new(&mut bytes))
        .map_err(|e| error(e.to_string()))?;
    bound(bytes.len())?;
    Ok(bytes)
}

fn decode_source(source: &[u8]) -> Result<Source, Error> {
    bound(source.len())?;
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
        if !(1..=VERSION).contains(&source.version) {
            return Err(error(format!(
                "unsupported module version {}",
                source.version
            )));
        }
        let env = Environment::new(&source)?;
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
        let canonical = encode_source(&source)?;
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        let native = (0..code.len()).map(|_| Default::default()).collect();
        Ok(Self {
            version: source.version,
            canonical: canonical.into(),
            interface,
            records: env.records,
            enums: env.enums,
            exports,
            code,
            #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
            native,
        })
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
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        if !changed.is_empty() {
            module.native = (0..self.code.len()).map(|_| Default::default()).collect();
        }
        for &index in changed {
            let function = &source.functions[index];
            module.code[index] = Arc::new(compile_body(&checker.env, function, &function.body)?);
        }
        if module.code.iter().map(|body| body.ops.len()).sum::<usize>() > MAX_CODE {
            return Err(error("module instruction count exceeds limit"));
        }
        module.canonical = encode_source(source)?.into();
        Ok(module)
    }

    pub(crate) fn canonical_source(&self) -> &[u8] {
        &self.canonical
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
        let module = Self::compile(decode_source(source)?)?;
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
}

impl<'a> BodyChecker<'a> {
    pub(crate) fn new(source: &'a Source) -> Result<Self, Error> {
        Ok(Self {
            env: Environment::new(source)?,
        })
    }

    pub(crate) fn check(&self, index: usize, body: &[Stmt]) -> Result<(), Error> {
        compile_body(&self.env, &self.env.functions[index], body).map(|_| ())
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
    functions: &'a [Function],
    function_ids: BTreeMap<(Option<String>, String), usize>,
    inputs: BTreeMap<String, (usize, Type)>,
    records: BTreeMap<String, Record>,
    enums: BTreeMap<String, Enum>,
}

impl<'a> Environment<'a> {
    fn new(source: &'a Source) -> Result<Self, Error> {
        if source.functions.is_empty()
            || source.functions.len() > MAX_DECLARATIONS
            || source.records.len() + source.enums.len() > MAX_DECLARATIONS
        {
            return Err(error("module declaration count outside limits"));
        }
        let mut env = Self {
            version: source.version,
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
                list_element(self.expr(list, next)?)?;
                (Op::Empty, Type::Bool)
            }
            Expr::Last { list } => (Op::Last, list_element(self.expr(list, next)?)?),
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
            Expr::Call { name, args } => self.call(None, name, args, next)?,
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
        if matches!(kind, And | Or) {
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
