//! WK1: an explicit, pinned common-word projection of checked structured source.
//!
//! A word is a deterministic typed recipe, never a new opcode or an unchecked
//! promise. Expansion preserves the ordinary FSM checker, fuel and trap order.
//! Dictionary installation and source expansion are separate bounded phases.
use super::{
    Binary, Binding, BodyChecker, Expr, Function, Module, Source, Stmt, Type, Unary, error,
};
use crate::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, Error>;
const MAX_DICTIONARY_BYTES: usize = 65_536;
const MAX_DEFINITIONS: usize = 64;
const MAX_RECIPE_NODES: usize = 8192;
const MAX_RECIPE_DEPTH: usize = 32;
const PRIVATE: &str = "wk_private_";

/// Exact scalar types supported by reusable recipes. Recipes cannot introduce
/// names, calls, host effects, ownership changes or unchecked native code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    Int,
    Uint,
    Float,
    Bool,
}

impl ScalarType {
    fn ty(self) -> Type {
        match self {
            Self::Int => Type::I64,
            Self::Uint => Type::U64,
            Self::Float => Type::F32,
            Self::Bool => Type::Bool,
        }
    }
}

/// A hygienic recipe. Parameters are indexed typed holes; there are no free
/// identifier strings. `Use` can refer only to an earlier dictionary entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Template {
    Param {
        index: usize,
    },
    Int {
        value: i64,
    },
    Uint {
        value: u64,
    },
    Float {
        bits: u32,
    },
    Bool {
        value: bool,
    },
    Binary {
        kind: Binary,
        left: Box<Template>,
        right: Box<Template>,
    },
    Unary {
        kind: Unary,
        value: Box<Template>,
    },
    Use {
        word: String,
        args: Vec<Template>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub word: String,
    pub params: Vec<ScalarType>,
    pub result: ScalarType,
    pub body: Template,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u16,
    definitions: Vec<Definition>,
}

#[derive(Clone)]
struct Recipe {
    params: Vec<ScalarType>,
    body: Template,
    uses: Vec<usize>,
}

/// An immutable checked dictionary. Its digest binds the complete canonical
/// definitions and WK1 semantics; a digest does not grant authority or establish
/// that a model knows the vocabulary. Hosts still choose which dictionary to use.
#[derive(Clone)]
pub struct Dictionary {
    identity: String,
    core: bool,
    definitions: Vec<Definition>,
    recipes: BTreeMap<String, Recipe>,
}

impl Dictionary {
    pub fn new(definitions: &[Definition]) -> Result<Self> {
        if definitions.is_empty() || definitions.len() > MAX_DEFINITIONS {
            return Err(error("WK1 dictionary definition limit"));
        }
        let mut raw_nodes = 0;
        let mut words = BTreeSet::new();
        for definition in definitions {
            // Preflight every externally sized field before cloning or encoding.
            check_word(&definition.word)?;
            if definition.params.is_empty()
                || definition.params.len() > 8
                || !words.insert(definition.word.as_str())
            {
                return Err(error("WK1 duplicate word or parameter limit"));
            }
            inspect_template(&definition.body, &mut raw_nodes)?;
        }
        let document = Document {
            version: 1,
            definitions: definitions.to_vec(),
        };
        let bytes = serde_json::to_vec(&document).map_err(|e| error(e.to_string()))?;
        if bytes.len() > MAX_DICTIONARY_BYTES {
            return Err(error("WK1 dictionary byte limit"));
        }
        let mut hasher = Sha256::new();
        hasher.update(b"FluxR WK1 dictionary v1\0");
        hasher.update(&bytes);
        let identity = format!("{:x}", hasher.finalize());
        let mut recipes = BTreeMap::new();
        let mut remaining = MAX_RECIPE_NODES;
        for definition in definitions {
            check_word(&definition.word)?;
            if definition.params.is_empty()
                || definition.params.len() > 8
                || recipes.contains_key(&definition.word)
            {
                return Err(error("WK1 duplicate word or parameter limit"));
            }
            let body = resolve(
                &definition.body,
                &definition.params,
                &recipes,
                &mut remaining,
                0,
            )?;
            if infer(&body, &definition.params)? != definition.result {
                return Err(error("WK1 recipe result type mismatch"));
            }
            let mut uses = vec![0; definition.params.len()];
            let mut first = Vec::new();
            visit_params(&body, &mut |index| {
                if uses[index] == 0 {
                    first.push(index);
                }
                uses[index] += 1;
            });
            if first != (0..definition.params.len()).collect::<Vec<_>>() {
                return Err(error(
                    "WK1 recipe must use every argument in declared first-use order",
                ));
            }
            // The released compiler independently checks the admitted recipe.
            let params: Vec<_> = definition
                .params
                .iter()
                .enumerate()
                .map(|(i, ty)| Binding {
                    name: format!("p{i}"),
                    ty: ty.ty(),
                })
                .collect();
            let args: Vec<_> = params
                .iter()
                .map(|p| Expr::Local {
                    name: p.name.clone(),
                })
                .collect();
            let mut budget = Budget::new(ExpansionLimits::default());
            let expr = instantiate(&body, &args, &mut budget, 0)?;
            Module::compile(Source {
                version: 2,
                inputs: vec![],
                records: vec![],
                enums: vec![],
                functions: vec![Function {
                    name: "recipe".into(),
                    receiver: None,
                    export: true,
                    params,
                    locals: vec![],
                    result: definition.result.ty(),
                    body: vec![Stmt::Return { value: expr }],
                }],
            })?;
            recipes.insert(
                definition.word.clone(),
                Recipe {
                    params: definition.params.clone(),
                    body,
                    uses,
                },
            );
        }
        Ok(Self {
            identity,
            core: false,
            definitions: definitions.to_vec(),
            recipes,
        })
    }

    /// Parse a bounded versioned dictionary document. Unknown keys, duplicate
    /// words, forward/recursive dependencies and ill-typed recipes refuse.
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > MAX_DICTIONARY_BYTES {
            return Err(error("WK1 dictionary byte limit"));
        }
        let document: Document = serde_json::from_str(text).map_err(|e| error(e.to_string()))?;
        if document.version != 1 {
            return Err(error("unsupported WK1 dictionary version"));
        }
        Self::new(&document.definitions)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&Document {
            version: 1,
            definitions: self.definitions.clone(),
        })
        .expect("checked scalar recipe serializes")
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn header(&self) -> &str {
        if self.core { "core" } else { &self.identity }
    }
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    /// Frozen WK1 core vocabulary. The `core` alias is bound by the WK1 profile;
    /// new meanings require a new profile or an explicit custom dictionary pin.
    pub fn core() -> Self {
        let mut dictionary = Self::new(&core_definitions()).expect("fixed WK1 core is checked");
        dictionary.core = true;
        dictionary
    }
}

/// Admission limits, separately from execution fuel/cells/frames. Lowering is
/// bounded even for exponentially reused dictionary DAGs. Limits may only tighten
/// the supported maxima; zero, unbounded and oversized policies refuse.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionLimits {
    pub source_bytes: usize,
    pub expanded_bytes: usize,
    pub nodes: usize,
    pub depth: usize,
}

impl Default for ExpansionLimits {
    fn default() -> Self {
        Self {
            source_bytes: 256 * 1024,
            expanded_bytes: 256 * 1024,
            nodes: 65_536,
            depth: 32,
        }
    }
}

impl ExpansionLimits {
    pub(super) fn validate(self) -> Result<()> {
        if self.source_bytes == 0
            || self.source_bytes > 256 * 1024
            || self.expanded_bytes == 0
            || self.expanded_bytes > 256 * 1024
            || self.nodes == 0
            || self.nodes > 65_536
            || self.depth == 0
            || self.depth > 32
        {
            return Err(error("invalid WK1 expansion limits"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionReport {
    pub profile: String,
    pub dictionary_sha256: String,
    pub dictionary_header: String,
    pub source_sha256: String,
    pub canonical_fsm_sha256: String,
    pub source_bytes: usize,
    pub expanded_agent_bytes: usize,
    /// Lexical whitespace words, explicitly not model tokens.
    pub source_words: usize,
    /// Conservative parse, substitution and obligation-copy work charged before
    /// allocation; this is not a VM instruction count.
    pub admission_nodes: usize,
    pub macro_calls: usize,
    pub generated_locals: usize,
}

pub struct Compiled {
    pub module: Module,
    pub expanded_agent: String,
    pub report: ExpansionReport,
}

/// Expand and admit WK1 against a host-selected immutable dictionary. The source
/// pin must agree with that dictionary. Output is ordinary checked FSM, so all
/// existing portable/prepared/JIT execution and ownership contracts still apply.
pub fn compile(text: &str, dictionary: &Dictionary, limits: ExpansionLimits) -> Result<Compiled> {
    let (source, context) = super::agent::parse_wordcode(text, dictionary, limits)?;
    let module = Module::compile(source)?;
    let expanded_agent = module.agent_text_limited(limits.expanded_bytes)?;
    let report = ExpansionReport {
        profile: "wk1".into(),
        dictionary_sha256: dictionary.identity().into(),
        dictionary_header: dictionary.header().into(),
        source_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        canonical_fsm_sha256: format!("{:x}", Sha256::digest(module.encode())),
        source_bytes: text.len(),
        expanded_agent_bytes: expanded_agent.len(),
        source_words: text.split_ascii_whitespace().count(),
        admission_nodes: context.budget.used,
        macro_calls: context.calls,
        generated_locals: context.total_locals,
    };
    Ok(Compiled {
        module,
        expanded_agent,
        report,
    })
}

impl Module {
    pub fn compile_wordcode(
        text: &str,
        dictionary: &Dictionary,
        limits: ExpansionLimits,
    ) -> Result<Self> {
        Ok(compile(text, dictionary, limits)?.module)
    }
}

struct Budget {
    limits: ExpansionLimits,
    used: usize,
}
impl Budget {
    fn new(limits: ExpansionLimits) -> Self {
        Self { limits, used: 0 }
    }
    fn charge(&mut self, count: usize, depth: usize) -> Result<()> {
        if depth >= self.limits.depth {
            return Err(error("WK1 expansion depth limit"));
        }
        self.used = self
            .used
            .checked_add(count)
            .ok_or_else(|| error("WK1 expansion node limit"))?;
        if self.used > self.limits.nodes {
            return Err(error("WK1 expansion node limit"));
        }
        Ok(())
    }
    fn copy(&mut self, expr: &Expr, depth: usize) -> Result<Expr> {
        let (count, extra) = expression_size(expr);
        self.charge(count, depth.saturating_add(extra))?;
        Ok(expr.clone())
    }
}

pub(super) struct Context<'a> {
    dictionary: &'a Dictionary,
    budget: Budget,
    calls: usize,
    total_locals: usize,
    function: usize,
    locals: Vec<Binding>,
    obligations: Vec<(usize, Expr, Type)>,
}

impl<'a> Context<'a> {
    pub(super) fn new(dictionary: &'a Dictionary, limits: ExpansionLimits) -> Self {
        Self {
            dictionary,
            budget: Budget::new(limits),
            calls: 0,
            total_locals: 0,
            function: 0,
            locals: vec![],
            obligations: vec![],
        }
    }
    pub(super) fn charge(&mut self, nodes: usize, depth: usize) -> Result<()> {
        self.budget.charge(nodes, depth)
    }
    pub(super) fn signature(&self, word: &str) -> Option<Vec<ScalarType>> {
        self.dictionary.recipes.get(word).map(|r| r.params.clone())
    }
    fn require_type(&mut self, expr: &Expr, ty: Type) -> Result<()> {
        let copy = self.budget.copy(expr, 0)?;
        self.obligations.push((self.function, copy, ty));
        Ok(())
    }
    pub(super) fn expression(&mut self, word: &str, args: Vec<Expr>, depth: usize) -> Result<Expr> {
        let dictionary = self.dictionary;
        let recipe = dictionary
            .recipes
            .get(word)
            .ok_or_else(|| error("WK1 unknown word"))?;
        for ((arg, ty), uses) in args.iter().zip(&recipe.params).zip(&recipe.uses) {
            if *uses > 1 && !atom(arg) {
                return Err(error(
                    "WK1 repeated hole requires a scalar atom; bind the expression once first",
                ));
            }
            self.require_type(arg, ty.ty())?;
        }
        self.calls += 1;
        instantiate(&recipe.body, &args, &mut self.budget, depth)
    }
    pub(super) fn finish_function(&mut self) -> Vec<Binding> {
        self.function += 1;
        std::mem::take(&mut self.locals)
    }
    pub(super) fn validate_arguments(&self, source: &Source) -> Result<()> {
        let checker = BodyChecker::new(source)?;
        for (function, expr, expected) in &self.obligations {
            if checker.expression_type(*function, expr)? != *expected {
                return Err(error("WK1 recipe argument type mismatch"));
            }
        }
        Ok(())
    }
    pub(super) fn repeat(
        &mut self,
        count: Expr,
        body: Vec<Stmt>,
        depth: usize,
    ) -> Result<Vec<Stmt>> {
        if self.locals.len() >= 256 {
            return Err(error("WK1 generated binding limit"));
        }
        self.charge(11, depth + 3)?;
        let name = format!("{PRIVATE}{}", self.locals.len());
        self.locals.push(Binding {
            name: name.clone(),
            ty: Type::I64,
        });
        self.total_locals += 1;
        self.calls += 1;
        let counter = || Expr::Local { name: name.clone() };
        let mut loop_body = Vec::with_capacity(body.len() + 1);
        // Decrement before the body so `continue` advances the private counter.
        loop_body.push(Stmt::Set {
            local: name.clone(),
            value: binary(Binary::Sub, counter(), int(1)),
        });
        loop_body.extend(body);
        Ok(vec![
            Stmt::Set {
                local: name.clone(),
                value: count,
            },
            Stmt::While {
                test: binary(Binary::Lt, int(0), counter()),
                body: loop_body,
            },
        ])
    }
    pub(super) fn statement(
        &mut self,
        word: &str,
        local: String,
        mut args: Vec<Expr>,
        depth: usize,
    ) -> Result<Stmt> {
        self.calls += 1;
        let value = Expr::Local {
            name: local.clone(),
        };
        self.require_type(&value, Type::I64)?;
        for arg in &args {
            self.require_type(arg, Type::I64)?;
        }
        match word {
            "next" => {
                self.charge(4, depth + 2)?;
                Ok(Stmt::Set {
                    local,
                    value: binary(Binary::Add, value, int(1)),
                })
            }
            "bump" | "scale" => {
                self.charge(3, depth + 2)?;
                let kind = if word == "bump" {
                    Binary::Add
                } else {
                    Binary::Mul
                };
                Ok(Stmt::Set {
                    local,
                    value: binary(kind, value, args.remove(0)),
                })
            }
            "walk" => {
                if args.iter().any(|arg| !atom(arg)) {
                    return Err(error("WK1 walk bound and increment require scalar atoms"));
                }
                self.charge(7, depth + 3)?;
                let end = args.remove(0);
                let step = args.remove(0);
                Ok(Stmt::While {
                    test: binary(Binary::Lt, value.clone(), end),
                    body: vec![Stmt::Set {
                        local,
                        value: binary(Binary::Add, value, step),
                    }],
                })
            }
            _ => Err(error("WK1 unknown statement word")),
        }
    }
}

pub(super) fn author_name(name: &str) -> Result<()> {
    if name.starts_with(PRIVATE) {
        Err(error("WK1 private identifier prefix is reserved"))
    } else {
        Ok(())
    }
}

fn atom(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Bool { .. }
            | Expr::I64 { .. }
            | Expr::U64 { .. }
            | Expr::F32 { .. }
            | Expr::Local { .. }
            | Expr::Input { .. }
    )
}
fn int(value: i64) -> Expr {
    Expr::I64 { value }
}
fn binary(kind: Binary, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn expression_size(expr: &Expr) -> (usize, usize) {
    let mut pending = vec![(expr, 0)];
    let (mut nodes, mut depth) = (0, 0);
    while let Some((expr, at)) = pending.pop() {
        nodes += 1;
        depth = depth.max(at);
        match expr {
            Expr::Binary { left, right, .. } => {
                pending.push((right, at + 1));
                pending.push((left, at + 1));
            }
            Expr::Unary { value, .. }
            | Expr::IsVariant { value, .. }
            | Expr::Payload { value, .. } => pending.push((value, at + 1)),
            Expr::Record { fields, .. } => pending.extend(fields.iter().map(|x| (x, at + 1))),
            Expr::Field { record, .. } => pending.push((record, at + 1)),
            Expr::Variant {
                payload: Some(p), ..
            } => pending.push((p, at + 1)),
            Expr::List { items, .. } => pending.extend(items.iter().map(|x| (x, at + 1))),
            Expr::Empty { list } | Expr::Last { list } | Expr::ArgMinF32 { list } => {
                pending.push((list, at + 1))
            }
            Expr::Index { list, index } => {
                pending.push((index, at + 1));
                pending.push((list, at + 1));
            }
            Expr::Call { args, .. } | Expr::HostCall { args, .. } => {
                pending.extend(args.iter().map(|x| (x, at + 1)))
            }
            Expr::Method { receiver, args, .. } => {
                pending.push((receiver, at + 1));
                pending.extend(args.iter().map(|x| (x, at + 1)));
            }
            _ => (),
        }
    }
    (nodes, depth)
}

fn check_word(word: &str) -> Result<()> {
    if word.is_empty()
        || word.len() > 16
        || !word.bytes().all(|b| b.is_ascii_lowercase())
        || RESERVED.split_ascii_whitespace().any(|x| x == word)
    {
        return Err(error(
            "WK1 word must be a nonreserved lowercase ASCII word of at most 16 bytes",
        ));
    }
    Ok(())
}
const RESERVED: &str = "wk1 fa1 core v1 v2 add sub mul div rem eq lt le and or not abs tof i64tof ftoi64 sqrt unit true false local input new field tag wrap is payload list empty last at minidx call m next bump scale walk repeat set put push pop if while break continue ret do inputs rec enum fn pub method int uint float bool record none";

fn inspect_template(body: &Template, nodes: &mut usize) -> Result<()> {
    let mut pending = vec![(body, 0)];
    while let Some((template, depth)) = pending.pop() {
        *nodes += 1;
        if *nodes > MAX_RECIPE_NODES || depth >= MAX_RECIPE_DEPTH {
            return Err(error("WK1 dictionary structural limit"));
        }
        match template {
            Template::Binary { left, right, .. } => {
                pending.push((right, depth + 1));
                pending.push((left, depth + 1));
            }
            Template::Unary { value, .. } => pending.push((value, depth + 1)),
            Template::Use { word, args } => {
                if args.len() > 8 {
                    return Err(error("WK1 recipe use arity limit"));
                }
                check_word(word)?;
                pending.extend(args.iter().map(|x| (x, depth + 1)));
            }
            _ => (),
        }
    }
    Ok(())
}

fn recipe_charge(remaining: &mut usize, depth: usize) -> Result<()> {
    if *remaining == 0 || depth >= MAX_RECIPE_DEPTH {
        return Err(error("WK1 expanded dictionary limit"));
    }
    *remaining -= 1;
    Ok(())
}

fn resolve(
    template: &Template,
    params: &[ScalarType],
    known: &BTreeMap<String, Recipe>,
    remaining: &mut usize,
    depth: usize,
) -> Result<Template> {
    recipe_charge(remaining, depth)?;
    Ok(match template {
        Template::Param { index } => {
            if *index >= params.len() {
                return Err(error("WK1 hole index out of range"));
            }
            template.clone()
        }
        Template::Binary { kind, left, right } => Template::Binary {
            kind: *kind,
            left: Box::new(resolve(left, params, known, remaining, depth + 1)?),
            right: Box::new(resolve(right, params, known, remaining, depth + 1)?),
        },
        Template::Unary { kind, value } => Template::Unary {
            kind: *kind,
            value: Box::new(resolve(value, params, known, remaining, depth + 1)?),
        },
        Template::Use { word, args } => {
            let recipe = known
                .get(word)
                .ok_or_else(|| error("WK1 unknown, recursive or forward recipe dependency"))?;
            if args.len() != recipe.params.len() {
                return Err(error("WK1 recipe use arity mismatch"));
            }
            let mut resolved = Vec::new();
            for (argument, expected) in args.iter().zip(&recipe.params) {
                let argument = resolve(argument, params, known, remaining, depth + 1)?;
                if infer(&argument, params)? != *expected {
                    return Err(error("WK1 nested recipe argument type mismatch"));
                }
                resolved.push(argument);
            }
            substitute(&recipe.body, &resolved, remaining, depth)?
        }
        _ => template.clone(),
    })
}

fn substitute(
    template: &Template,
    args: &[Template],
    remaining: &mut usize,
    depth: usize,
) -> Result<Template> {
    recipe_charge(remaining, depth)?;
    match template {
        Template::Param { index } => copy_template(&args[*index], remaining, depth),
        Template::Binary { kind, left, right } => Ok(Template::Binary {
            kind: *kind,
            left: Box::new(substitute(left, args, remaining, depth + 1)?),
            right: Box::new(substitute(right, args, remaining, depth + 1)?),
        }),
        Template::Unary { kind, value } => Ok(Template::Unary {
            kind: *kind,
            value: Box::new(substitute(value, args, remaining, depth + 1)?),
        }),
        Template::Use { .. } => Err(error("WK1 unresolved recipe invariant")),
        _ => Ok(template.clone()),
    }
}
fn copy_template(template: &Template, remaining: &mut usize, depth: usize) -> Result<Template> {
    recipe_charge(remaining, depth)?;
    Ok(match template {
        Template::Binary { kind, left, right } => Template::Binary {
            kind: *kind,
            left: Box::new(copy_template(left, remaining, depth + 1)?),
            right: Box::new(copy_template(right, remaining, depth + 1)?),
        },
        Template::Unary { kind, value } => Template::Unary {
            kind: *kind,
            value: Box::new(copy_template(value, remaining, depth + 1)?),
        },
        Template::Use { .. } => return Err(error("WK1 unresolved recipe invariant")),
        _ => template.clone(),
    })
}

fn infer(template: &Template, params: &[ScalarType]) -> Result<ScalarType> {
    use ScalarType::{Bool, Float, Int, Uint};
    Ok(match template {
        Template::Param { index } => *params
            .get(*index)
            .ok_or_else(|| error("WK1 hole index out of range"))?,
        Template::Int { .. } => Int,
        Template::Uint { .. } => Uint,
        Template::Float { .. } => Float,
        Template::Bool { .. } => Bool,
        Template::Unary { kind, value } => {
            let (expected, result) = match kind {
                Unary::Not => (Bool, Bool),
                Unary::AbsF32 | Unary::SqrtF32 => (Float, Float),
                Unary::U64ToF32 => (Uint, Float),
                Unary::I64ToF32 => (Int, Float),
                Unary::F32ToI64 => (Float, Int),
            };
            if infer(value, params)? != expected {
                return Err(error("WK1 unary recipe type mismatch"));
            }
            result
        }
        Template::Binary { kind, left, right } => {
            let left = infer(left, params)?;
            if infer(right, params)? != left {
                return Err(error("WK1 binary recipe type mismatch"));
            }
            match kind {
                Binary::And | Binary::Or if left == Bool => Bool,
                Binary::Eq => Bool,
                Binary::Lt | Binary::Le if left != Bool => Bool,
                Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                    if left != Bool =>
                {
                    left
                }
                _ => return Err(error("WK1 unsupported binary recipe types")),
            }
        }
        Template::Use { .. } => return Err(error("WK1 unresolved recipe invariant")),
    })
}

fn visit_params(template: &Template, visitor: &mut impl FnMut(usize)) {
    match template {
        Template::Param { index } => visitor(*index),
        Template::Binary { left, right, .. } => {
            visit_params(left, visitor);
            visit_params(right, visitor);
        }
        Template::Unary { value, .. } => visit_params(value, visitor),
        _ => (),
    }
}

fn instantiate(
    template: &Template,
    args: &[Expr],
    budget: &mut Budget,
    depth: usize,
) -> Result<Expr> {
    budget.charge(1, depth)?;
    Ok(match template {
        Template::Param { index } => return budget.copy(&args[*index], depth),
        Template::Int { value } => Expr::I64 { value: *value },
        Template::Uint { value } => Expr::U64 { value: *value },
        Template::Float { bits } => Expr::F32 { bits: *bits },
        Template::Bool { value } => Expr::Bool { value: *value },
        Template::Binary { kind, left, right } => binary(
            *kind,
            instantiate(left, args, budget, depth + 1)?,
            instantiate(right, args, budget, depth + 1)?,
        ),
        Template::Unary { kind, value } => Expr::Unary {
            kind: *kind,
            value: Box::new(instantiate(value, args, budget, depth + 1)?),
        },
        Template::Use { .. } => return Err(error("WK1 unresolved recipe invariant")),
    })
}

fn core_definitions() -> Vec<Definition> {
    use Binary::{Add, And, Div, Eq, Le, Lt, Mul, Sub};
    use ScalarType::{Bool, Float, Int};
    let p = |index| Template::Param { index };
    let n = |value| Template::Int { value };
    let b = |kind, left, right| Template::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    };
    let d = |word: &str, params, result, body| Definition {
        word: word.into(),
        params,
        result,
        body,
    };
    vec![
        d("up", vec![Int], Int, b(Add, p(0), n(1))),
        d("down", vec![Int], Int, b(Sub, p(0), n(1))),
        d("twice", vec![Int], Int, b(Mul, p(0), n(2))),
        d("square", vec![Int], Int, b(Mul, p(0), p(0))),
        d("cube", vec![Int], Int, b(Mul, b(Mul, p(0), p(0)), p(0))),
        d(
            "step",
            vec![Int, Int, Int],
            Int,
            b(Add, b(Mul, p(0), p(1)), p(2)),
        ),
        d(
            "sum",
            vec![Int, Int, Int],
            Int,
            b(Add, b(Add, p(0), p(1)), p(2)),
        ),
        d(
            "ratio",
            vec![Int, Int, Int],
            Int,
            b(Div, b(Add, p(0), p(1)), p(2)),
        ),
        d("zero", vec![Int], Bool, b(Eq, p(0), n(0))),
        d("positive", vec![Int], Bool, b(Lt, n(0), p(0))),
        d("negative", vec![Int], Bool, b(Lt, p(0), n(0))),
        d(
            "between",
            vec![Int, Int, Int],
            Bool,
            b(And, b(Le, p(0), p(1)), b(Lt, p(1), p(2))),
        ),
        d(
            "four",
            vec![Int],
            Int,
            Template::Use {
                word: "twice".into(),
                args: vec![Template::Use {
                    word: "twice".into(),
                    args: vec![p(0)],
                }],
            },
        ),
        d(
            "blend",
            vec![Float, Float, Float],
            Float,
            b(Add, b(Mul, p(0), p(1)), p(2)),
        ),
        d(
            "root",
            vec![Float],
            Float,
            Template::Unary {
                kind: Unary::SqrtF32,
                value: Box::new(Template::Unary {
                    kind: Unary::AbsF32,
                    value: Box::new(p(0)),
                }),
            },
        ),
    ]
}
