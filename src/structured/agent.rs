//! FA1: an optional whitespace-delimited authoring projection, not a new ABI.
//!
//! Fixed-arity prefix operators remove structural punctuation. Variable-arity
//! forms carry bounded counts, so parsing does not consult mutable declarations.
//! Every route still uses the released structured checker and canonical encoder.
use super::{
    Binary, Binding, Enum, Expr, Function, Module, Record, Source, Stmt, Type, Unary, error,
};
use crate::{Error, bound};
use std::str::SplitAsciiWhitespace;

#[derive(Clone)]
enum Words<'a> {
    Legacy(SplitAsciiWhitespace<'a>),
    Unified(std::iter::Filter<std::str::Split<'a, fn(char) -> bool>, fn(&&str) -> bool>),
}
impl<'a> Iterator for Words<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Legacy(words) => words.next(),
            Self::Unified(words) => words.next(),
        }
    }
}
fn unified_words(text: &str) -> Words<'_> {
    fn whitespace(c: char) -> bool {
        c.is_ascii() && super::unified_source::space_byte(c as u8)
    }
    fn nonempty(word: &&str) -> bool {
        !word.is_empty()
    }
    Words::Unified(
        text.split(whitespace as fn(char) -> bool)
            .filter(nonempty as fn(&&str) -> bool),
    )
}

type Result<T> = std::result::Result<T, Error>;
const MAX_DEPTH: usize = 96;
const MAX_NODES: usize = 65_536;
const MAX_ITEMS: usize = 32_768;
const MAX_BINDINGS: usize = 256;
const MAX_DECLARATIONS: usize = 512;
const MAX_DECIMAL_FLOAT_BYTES: usize = 256;

impl Source {
    /// Parse the explicitly versioned FA1 source projection. Semantic checking is
    /// separate, exactly as with `parse_compact`; use `Module::compile_agent` to
    /// admit a program. No declaration lookup is needed to parse an expression.
    pub fn parse_agent(text: &str) -> Result<Self> {
        let mut p = Parser::new(text)?;
        p.expect("fa1")?;
        p.source()
    }
}

pub(super) fn parse_wordcode<'a>(
    text: &'a str,
    dictionary: &'a super::wordcode::Dictionary,
    limits: super::wordcode::ExpansionLimits,
) -> Result<(Source, super::wordcode::Context<'a>)> {
    limits.validate()?;
    if text.len() > limits.source_bytes {
        return Err(error("WK1 source byte limit"));
    }
    let mut parser = Parser::new(text)?;
    parser.expect("wk1")?;
    if parser.word()? != dictionary.header() {
        return Err(error("WK1 dictionary pin mismatch"));
    }
    parser.wordcode = Some(super::wordcode::Context::new(dictionary, limits));
    let source = parser.source()?;
    let context = parser.wordcode.take().unwrap();
    context.validate_arguments(&source)?;
    Ok((source, context))
}

impl Module {
    /// Compile FA1 through the existing structured checker and execution tiers.
    pub fn compile_agent(text: &str) -> Result<Self> {
        Self::compile(Source::parse_agent(text)?)
    }

    /// Render FA1 and verify byte-for-byte equality of the released FSM payload.
    /// Output allocation is bounded before append, even when larger than input.
    pub fn agent_text(&self) -> Result<String> {
        self.agent_text_limited(256 * 1024)
    }

    pub(super) fn agent_text_limited(&self, limit: usize) -> Result<String> {
        self.prefix_text_limited(limit, false)
    }

    /// The only current language header. Older projections are import codecs.
    pub fn unified_text(&self) -> Result<String> {
        self.prefix_text_limited(256 * 1024, true)
    }

    fn prefix_text_limited(&self, limit: usize, unified: bool) -> Result<String> {
        let source = self.checked_source()?;
        let mut out = Printer::new(limit);
        out.unified = unified;
        out.token(if unified { "fluxr" } else { "fa1" })?;
        out.token(if unified {
            "1"
        } else if source.version == 1 {
            "v1"
        } else {
            "v2"
        })?;
        if !source.inputs.is_empty() {
            out.line()?;
            out.token("inputs")?;
            out.bindings(&source.inputs)?;
        }
        for record in &source.records {
            out.line()?;
            out.token("rec")?;
            out.token(&record.name)?;
            out.bindings(&record.fields)?;
        }
        for enumeration in &source.enums {
            out.line()?;
            out.token("enum")?;
            out.token(&enumeration.name)?;
            out.count(enumeration.variants.len())?;
            for variant in &enumeration.variants {
                out.token(&variant.name)?;
                if let Some(kind) = &variant.payload {
                    out.ty(kind)?;
                } else {
                    out.token("none")?;
                }
            }
        }
        for function in &source.functions {
            out.line()?;
            if let Some(receiver) = &function.receiver {
                out.token("method")?;
                out.token(receiver)?;
            } else {
                out.token(if function.export { "pub" } else { "fn" })?;
            }
            out.token(&function.name)?;
            out.bindings(&function.params)?;
            out.ty(&function.result)?;
            if function.locals.is_empty()
                && let [Stmt::Return { value }] = &function.body[..]
            {
                out.token("ret")?;
                out.expr(value)?;
            } else {
                out.token("do")?;
                out.bindings(&function.locals)?;
                out.block(&function.body)?;
            }
        }
        let mut text = out.text;
        if unified && text == "fluxr 1" {
            text.push('\n');
        }
        let reparsed = if unified {
            super::SourceDocument::parse(&text)?.source
        } else {
            Source::parse_agent(&text)?
        };
        let mut expected = source;
        if unified {
            expected.version = 2;
        }
        if reparsed != expected
            || (!unified && super::compile::encode_source(&reparsed)? != self.canonical_source())
        {
            return Err(error("FA1 projection changed canonical source"));
        }
        Ok(text)
    }
}

pub(super) fn parse_unified(text: &str) -> Result<super::SourceDocument> {
    use sha2::{Digest, Sha256};
    if text.len() > super::unified_source::MAX_SOURCE_BYTES {
        return Err(error("FluxR source exceeds byte limit"));
    }
    if !text.starts_with("fluxr 1\n") {
        return Err(error(
            "expected exact fluxr 1 header; use import for legacy encodings",
        ));
    }
    let mut parser = Parser {
        text,
        words: unified_words(&text[8..]),
        nodes: 0,
        wordcode: None,
        unified: true,
        native_spans: Vec::new(),
        checked_fragments: Vec::new(),
    };
    let source = parser.declarations(2)?;
    // A mixed document may contain much more native source than checked text.
    // Admit the complete checked projection before allocating or joining it.
    let checked_bytes = parser
        .checked_fragments
        .iter()
        .fold("fluxr 1\n".len(), |length, fragment| {
            length.saturating_add(fragment.len())
        })
        .saturating_add(parser.checked_fragments.len().saturating_sub(1));
    bound(checked_bytes)?;
    let mut checked_text = String::with_capacity(checked_bytes);
    checked_text.push_str("fluxr 1\n");
    for (index, fragment) in parser.checked_fragments.iter().enumerate() {
        if index != 0 {
            checked_text.push('\n');
        }
        checked_text.push_str(fragment);
    }
    let mut hash = Sha256::new();
    hash.update(b"fluxr-native-bindings-v1\0");
    for span in &parser.native_spans {
        let bytes = &text.as_bytes()[span.start..span.end];
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(super::SourceDocument {
        source,
        checked_text,
        native_spans: parser.native_spans,
        native_digest: format!("{:x}", hash.finalize()),
    })
}

impl Stmt {
    /// Parse an independent edit body: `fa1 body COUNT STMT...`.
    /// The caller supplies normal collaboration pins/targets; this is only syntax.
    pub fn parse_agent_body(text: &str) -> Result<Vec<Self>> {
        let mut p = Parser::new(text)?;
        p.expect("fa1")?;
        p.expect("body")?;
        let body = p.block(0)?;
        if p.words.next().is_some() {
            return Err(error("trailing FA1 edit body tokens"));
        }
        Ok(body)
    }
}

struct Parser<'a> {
    text: &'a str,
    words: Words<'a>,
    nodes: usize,
    wordcode: Option<super::wordcode::Context<'a>>,
    unified: bool,
    native_spans: Vec<super::NativeItemSpan>,
    checked_fragments: Vec<&'a str>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self> {
        bound(text.len())?;
        if !text.is_ascii() {
            return Err(error("FA1 source must be ASCII"));
        }
        Ok(Self {
            text,
            words: Words::Legacy(text.split_ascii_whitespace()),
            nodes: 0,
            wordcode: None,
            unified: false,
            native_spans: Vec::new(),
            checked_fragments: Vec::new(),
        })
    }
    fn source(&mut self) -> Result<Source> {
        let version = match self.word()? {
            "v1" => 1,
            "v2" => 2,
            _ => return Err(error("FA1 requires module version v1 or v2")),
        };
        self.declarations(version)
    }
    fn declarations(&mut self, version: u16) -> Result<Source> {
        let mut source = Source {
            version,
            inputs: vec![],
            records: vec![],
            enums: vec![],
            functions: vec![],
        };
        while let Some(tag) = self.words.next() {
            let start = tag.as_ptr() as usize - self.text.as_ptr() as usize;
            self.node(0)?;
            if self.unified && super::unified_source::is_native_tag(tag) {
                let end = super::unified_source::skip_native(self.text, start, &mut self.nodes)?;
                self.native_spans.push(super::NativeItemSpan { start, end });
                self.words = unified_words(&self.text[end..]);
                continue;
            }
            match tag {
                "inputs" => {
                    let inputs = self.bindings()?;
                    if source.inputs.len() + inputs.len() > MAX_BINDINGS {
                        return Err(error("FA1 input count exceeds limit"));
                    }
                    source.inputs.extend(inputs);
                }
                "rec" => {
                    let name = self.name()?;
                    let fields = self.bindings()?;
                    source.records.push(Record { name, fields });
                }
                "enum" => {
                    let name = self.name()?;
                    let count = self.count(MAX_BINDINGS)?;
                    let mut variants = Vec::new();
                    for _ in 0..count {
                        let name = self.name()?;
                        let token = self.word()?;
                        let payload = if token == "none" {
                            None
                        } else {
                            Some(self.ty_token(token, 0)?)
                        };
                        variants.push(super::Variant { name, payload });
                    }
                    source.enums.push(Enum { name, variants });
                }
                "fn" | "pub" => source.functions.push(self.function(tag == "pub", None)?),
                "method" => {
                    let receiver = self.name()?;
                    source.functions.push(self.function(false, Some(receiver))?);
                }
                _ => return Err(error(format!("unknown FA1 declaration {tag}"))),
            }
            if self.unified {
                let end = self.words.clone().next().map_or(self.text.len(), |word| {
                    word.as_ptr() as usize - self.text.as_ptr() as usize
                });
                self.checked_fragments
                    .push(self.text[start..end].trim_end());
            }
            if source.functions.len() > MAX_DECLARATIONS
                || source.records.len() + source.enums.len() > MAX_DECLARATIONS
            {
                return Err(error("FA1 declaration count exceeds limit"));
            }
        }
        Ok(source)
    }
    fn node(&mut self, depth: usize) -> Result<()> {
        if let Some(context) = &mut self.wordcode {
            context.charge(1, depth)?;
        }
        if depth >= MAX_DEPTH {
            return Err(error("FA1 nesting exceeds limit"));
        }
        if self.nodes >= MAX_NODES {
            return Err(error("FA1 node count exceeds limit"));
        }
        self.nodes += 1;
        Ok(())
    }
    fn word(&mut self) -> Result<&'a str> {
        self.words
            .next()
            .ok_or_else(|| error("unexpected end of FA1 source"))
    }
    fn expect(&mut self, expected: &str) -> Result<()> {
        if self.word()? != expected {
            return Err(error(format!("expected {expected} in FA1 source")));
        }
        Ok(())
    }
    fn count(&mut self, limit: usize) -> Result<usize> {
        let token = self.word()?;
        if token.is_empty()
            || !token.bytes().all(|b| b.is_ascii_digit())
            || (token.len() > 1 && token.starts_with('0'))
        {
            return Err(error(
                "FA1 counts require canonical nonnegative decimal integers",
            ));
        }
        let count = token
            .parse::<usize>()
            .map_err(|_| error("FA1 count exceeds limit"))?;
        if count > limit {
            return Err(error("FA1 count exceeds limit"));
        }
        Ok(count)
    }
    fn name(&mut self) -> Result<String> {
        let token = self.word()?;
        name(token)?;
        if self.wordcode.is_some() {
            super::wordcode::author_name(token)?;
        }
        Ok(token.into())
    }
    fn bindings(&mut self) -> Result<Vec<Binding>> {
        let count = self.count(MAX_BINDINGS)?;
        // Never reserve an attacker-supplied count before reading its contents.
        let mut bindings = Vec::new();
        for _ in 0..count {
            self.node(0)?;
            bindings.push(Binding {
                name: self.name()?,
                ty: self.ty(0)?,
            });
        }
        Ok(bindings)
    }
    fn ty(&mut self, depth: usize) -> Result<Type> {
        let token = self.word()?;
        self.ty_token(token, depth)
    }
    fn ty_token(&mut self, token: &str, depth: usize) -> Result<Type> {
        self.node(depth)?;
        if depth >= 32 {
            return Err(error("FA1 type nesting exceeds limit"));
        }
        Ok(match token {
            "unit" => Type::Unit,
            "bool" => Type::Bool,
            "int" => Type::I64,
            "uint" => Type::U64,
            "float" => Type::F32,
            "record" => Type::Record(self.name()?),
            "enum" => Type::Enum(self.name()?),
            "list" => Type::List(Box::new(self.ty(depth + 1)?)),
            _ => return Err(error(format!("unknown FA1 type {token}"))),
        })
    }
    fn function(&mut self, export: bool, receiver: Option<String>) -> Result<Function> {
        let name = self.name()?;
        let params = self.bindings()?;
        let result = self.ty(0)?;
        let (mut locals, body) = match self.word()? {
            "ret" => (
                vec![],
                vec![Stmt::Return {
                    value: self.expr(0)?,
                }],
            ),
            "do" => (self.bindings()?, self.block(0)?),
            _ => {
                return Err(error(
                    "FA1 function requires ret expression or do locals block",
                ));
            }
        };
        if let Some(context) = &mut self.wordcode {
            locals.extend(context.finish_function());
            if params.len() + locals.len() > MAX_BINDINGS {
                return Err(error("WK1 expanded binding count exceeds limit"));
            }
        }
        Ok(Function {
            name,
            receiver,
            export,
            params,
            result,
            locals,
            body,
        })
    }
    fn block(&mut self, depth: usize) -> Result<Vec<Stmt>> {
        self.node(depth)?;
        let count = self.count(MAX_ITEMS)?;
        let mut statements = Vec::new();
        for _ in 0..count {
            if self.wordcode.is_some() && self.words.clone().next() == Some("repeat") {
                self.word()?;
                self.node(depth + 1)?;
                let count = self.expr(depth + 2)?;
                let body = self.block(depth + 2)?;
                statements.extend(self.wordcode.as_mut().unwrap().repeat(
                    count,
                    body,
                    depth + 1,
                )?);
            } else {
                statements.push(self.stmt(depth + 1)?);
            }
        }
        Ok(statements)
    }
    fn stmt(&mut self, depth: usize) -> Result<Stmt> {
        self.node(depth)?;
        let tag = self.word()?;
        if self.wordcode.is_some() && matches!(tag, "next" | "bump" | "scale" | "walk") {
            let local = self.name()?;
            let mut args = Vec::new();
            for _ in 0..match tag {
                "next" => 0,
                "walk" => 2,
                _ => 1,
            } {
                args.push(self.expr(depth + 1)?);
            }
            return self
                .wordcode
                .as_mut()
                .unwrap()
                .statement(tag, local, args, depth);
        }
        Ok(match tag {
            "next" | "prev" if self.unified => {
                let local = self.name()?;
                // Unit steps are the signed literal 1, not type-directed sugar.
                // Charge exactly `set x add/sub local x 1`, including its leaf.
                self.node(depth + 1)?; // Binary
                self.node(depth + 2)?; // Local
                self.node(depth + 2)?; // I64
                Stmt::Set {
                    value: Expr::Binary {
                        kind: if tag == "next" {
                            Binary::Add
                        } else {
                            Binary::Sub
                        },
                        left: Box::new(Expr::Local {
                            name: local.clone(),
                        }),
                        right: Box::new(Expr::I64 { value: 1 }),
                    },
                    local,
                }
            }
            "bump" | "scale" | "sub" | "div" | "rem" if self.unified => {
                let local = self.name()?;
                // Exactly the nodes and depths in `set x op local x rhs`.
                // The spelling cannot buy a larger syntax or execution budget.
                self.node(depth + 1)?; // Binary
                self.node(depth + 2)?; // Local, evaluated before the right side
                let right = self.boxed(depth + 2)?;
                Stmt::Set {
                    value: Expr::Binary {
                        kind: match tag {
                            "bump" => Binary::Add,
                            "scale" => Binary::Mul,
                            "sub" => Binary::Sub,
                            "div" => Binary::Div,
                            "rem" => Binary::Rem,
                            _ => unreachable!(),
                        },
                        left: Box::new(Expr::Local {
                            name: local.clone(),
                        }),
                        right,
                    },
                    local,
                }
            }
            "set" => Stmt::Set {
                local: self.name()?,
                value: self.expr(depth + 1)?,
            },
            "put" => Stmt::SetField {
                local: self.name()?,
                field: self.name()?,
                value: self.expr(depth + 1)?,
            },
            "push" => Stmt::Push {
                local: self.name()?,
                value: self.expr(depth + 1)?,
            },
            "pop" => Stmt::Pop {
                local: self.name()?,
            },
            "if" => Stmt::If {
                test: self.expr(depth + 1)?,
                then: self.block(depth + 1)?,
                otherwise: self.block(depth + 1)?,
            },
            "while" => Stmt::While {
                test: self.expr(depth + 1)?,
                body: self.block(depth + 1)?,
            },
            "break" => Stmt::Break,
            "continue" => Stmt::Continue,
            "ret" => Stmt::Return {
                value: self.expr(depth + 1)?,
            },
            tag => return Err(error(format!("unknown FA1 statement {tag}"))),
        })
    }
    fn boxed(&mut self, depth: usize) -> Result<Box<Expr>> {
        Ok(Box::new(self.expr(depth)?))
    }
    fn expressions(&mut self, depth: usize) -> Result<Vec<Expr>> {
        let count = self.count(MAX_ITEMS)?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(self.expr(depth)?);
        }
        Ok(values)
    }
    fn expr(&mut self, depth: usize) -> Result<Expr> {
        self.node(depth)?;
        let token = self.word()?;
        if let Some(kind) = binary(token) {
            return Ok(Expr::Binary {
                kind,
                left: self.boxed(depth + 1)?,
                right: self.boxed(depth + 1)?,
            });
        }
        if let Some(kind) = unary(token) {
            return Ok(Expr::Unary {
                kind,
                value: self.boxed(depth + 1)?,
            });
        }
        if let Some(signature) = self
            .wordcode
            .as_ref()
            .and_then(|context| context.signature(token))
        {
            let mut args = Vec::new();
            for _ in &signature {
                args.push(self.expr(depth + 1)?);
            }
            return self
                .wordcode
                .as_mut()
                .unwrap()
                .expression(token, args, depth);
        }
        if self.wordcode.is_some() {
            super::wordcode::author_name(token)?;
        }
        Ok(match token {
            "unit" => Expr::Unit,
            "true" => Expr::Bool { value: true },
            "false" => Expr::Bool { value: false },
            "local" => Expr::Local { name: self.name()? },
            "input" => Expr::Input { name: self.name()? },
            "new" => Expr::Record {
                name: self.name()?,
                fields: self.expressions(depth + 1)?,
            },
            "field" => Expr::Field {
                record: self.boxed(depth + 1)?,
                name: self.name()?,
            },
            "tag" | "wrap" => Expr::Variant {
                name: self.name()?,
                variant: self.name()?,
                payload: if token == "wrap" {
                    Some(self.boxed(depth + 1)?)
                } else {
                    None
                },
            },
            "is" => Expr::IsVariant {
                value: self.boxed(depth + 1)?,
                variant: self.name()?,
            },
            "payload" => Expr::Payload {
                value: self.boxed(depth + 1)?,
                variant: self.name()?,
            },
            "list" => Expr::List {
                element: self.ty(0)?,
                items: self.expressions(depth + 1)?,
            },
            "empty" => Expr::Empty {
                list: self.boxed(depth + 1)?,
            },
            "last" => Expr::Last {
                list: self.boxed(depth + 1)?,
            },
            "at" => Expr::Index {
                list: self.boxed(depth + 1)?,
                index: self.boxed(depth + 1)?,
            },
            "minidx" => Expr::ArgMinF32 {
                list: self.boxed(depth + 1)?,
            },
            "call" => Expr::Call {
                name: self.name()?,
                args: self.expressions(depth + 1)?,
            },
            "host" if self.unified => {
                let name = self.name()?;
                let result = self.ty(0)?;
                let count = self.count(MAX_BINDINGS)?;
                let mut params = Vec::with_capacity(count);
                let mut args = Vec::with_capacity(count);
                for _ in 0..count {
                    params.push(self.ty(0)?);
                    args.push(self.expr(depth + 1)?);
                }
                Expr::HostCall {
                    name,
                    params,
                    result,
                    args,
                }
            }
            "m" => Expr::Method {
                receiver: self.boxed(depth + 1)?,
                name: self.name()?,
                args: self.expressions(depth + 1)?,
            },
            _ => atom(token, self.unified)?,
        })
    }
}

fn name(token: &str) -> Result<()> {
    let mut bytes = token.bytes();
    if token.len() > 64
        || !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(error("invalid FA1 identifier"));
    }
    Ok(())
}

fn atom(token: &str, unified: bool) -> Result<Expr> {
    if token
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_digit() || *b == b'-')
    {
        if let Some(value) = token.strip_suffix('u') {
            return Ok(Expr::U64 {
                value: value
                    .parse()
                    .map_err(|_| error("invalid FA1 unsigned literal"))?,
            });
        }
        if let Some(bits) = token.strip_prefix("0x").and_then(|s| s.strip_suffix('f')) {
            return Ok(Expr::F32 {
                bits: u32::from_str_radix(bits, 16).map_err(|_| error("invalid FA1 float bits"))?,
            });
        }
        if unified && let Some(decimal) = token.strip_suffix('f') {
            return Ok(Expr::F32 {
                bits: decimal_float_bits(decimal)?,
            });
        }
        return Ok(Expr::I64 {
            value: token
                .parse()
                .map_err(|_| error("invalid FA1 integer literal"))?,
        });
    }
    name(token)?;
    Ok(Expr::Local { name: token.into() })
}

// Only current-source numeric atoms reach this path. In particular, spellings
// such as `nanf` and `inff` remain ordinary identifiers, and FA1/WK1 retain their
// exact released readers. Decimal conversion is to binary32 directly, avoiding
// an intermediate binary64 value and its possible double rounding.
fn decimal_float_bits(decimal: &str) -> Result<u32> {
    // Every finite binary32 value has an exact decimal spelling within this
    // bound. Refuse oversized new spellings before numeric conversion work;
    // the independently released raw-bit reader is unaffected.
    if decimal.len().saturating_add(1) > MAX_DECIMAL_FLOAT_BYTES {
        return Err(error("FluxR decimal float literal byte limit"));
    }
    let bytes = decimal.as_bytes();
    let mut position = usize::from(bytes.first() == Some(&b'-'));
    let digits = |position: &mut usize| {
        let start = *position;
        while bytes.get(*position).is_some_and(u8::is_ascii_digit) {
            *position += 1;
        }
        *position != start
    };
    let invalid = || error("invalid FluxR decimal float literal");
    if !digits(&mut position) {
        return Err(invalid());
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        if !digits(&mut position) {
            return Err(invalid());
        }
    }
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        if matches!(bytes.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        if !digits(&mut position) {
            return Err(invalid());
        }
    }
    if position != bytes.len() {
        return Err(invalid());
    }
    let value = decimal.parse::<f32>().map_err(|_| invalid())?;
    if !value.is_finite() {
        return Err(error("FluxR decimal float literal overflows binary32"));
    }
    Ok(value.to_bits())
}

// Shortest binary32 formatting needs at most nine significant digits, with
// decimal exponent -45..=38. Sixty-four bytes accommodate fixed/scientific
// notation, its sign and suffix, without allocating temporary strings.
struct FloatLiteral {
    bytes: [u8; 64],
    len: usize,
}

impl FloatLiteral {
    fn formatted(arguments: std::fmt::Arguments<'_>) -> Result<Self> {
        let mut literal = Self {
            bytes: [0; 64],
            len: 0,
        };
        std::fmt::write(&mut literal, arguments)
            .map_err(|_| error("binary32 projection formatting limit"))?;
        Ok(literal)
    }

    fn as_str(&self) -> &str {
        // write_str appends complete UTF-8 strings or leaves the buffer intact.
        std::str::from_utf8(&self.bytes[..self.len]).expect("complete UTF-8 formatting writes")
    }

    fn suffix(&mut self) -> Result<()> {
        std::fmt::Write::write_str(self, "f")
            .map_err(|_| error("binary32 projection formatting limit"))
    }
}

impl std::fmt::Write for FloatLiteral {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(std::fmt::Error)?;
        let target = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn unified_float_literal(bits: u32) -> Result<FloatLiteral> {
    let mut best = FloatLiteral::formatted(format_args!("0x{bits:x}f"))?;
    let value = f32::from_bits(bits);
    // Decimal notation cannot carry infinities or a NaN's payload/sign. Keep
    // the exact raw representation, including signaling NaNs, in that case.
    if value.is_finite() {
        let mut decimal = FloatLiteral::formatted(format_args!("{value}"))?;
        if decimal.len + 1 < best.len
            && decimal_float_bits(decimal.as_str()).is_ok_and(|roundtrip| roundtrip == bits)
        {
            decimal.suffix()?;
            best = decimal;
        }
        // Even a one-digit scientific literal needs a mantissa, 'e', an
        // exponent digit and 'f', plus these signs. If that lower bound cannot
        // beat the current winner, preserve its exact tie-breaking choice.
        let scientific_minimum = 4
            + usize::from(value.is_sign_negative())
            + usize::from(value != 0.0 && value.abs() < 1.0);
        if scientific_minimum < best.len {
            let mut scientific = FloatLiteral::formatted(format_args!("{value:e}"))?;
            if scientific.len + 1 < best.len
                && decimal_float_bits(scientific.as_str()).is_ok_and(|roundtrip| roundtrip == bits)
            {
                scientific.suffix()?;
                best = scientific;
            }
        }
    }
    Ok(best)
}

fn binary(token: &str) -> Option<Binary> {
    Some(match token {
        "add" => Binary::Add,
        "sub" => Binary::Sub,
        "mul" => Binary::Mul,
        "div" => Binary::Div,
        "rem" => Binary::Rem,
        "eq" => Binary::Eq,
        "lt" => Binary::Lt,
        "le" => Binary::Le,
        "and" => Binary::And,
        "or" => Binary::Or,
        _ => return None,
    })
}
fn unary(token: &str) -> Option<Unary> {
    Some(match token {
        "not" => Unary::Not,
        "abs" => Unary::AbsF32,
        "tof" => Unary::U64ToF32,
        "i64tof" => Unary::I64ToF32,
        "ftoi64" => Unary::F32ToI64,
        "sqrt" => Unary::SqrtF32,
        _ => return None,
    })
}
fn reserved(token: &str) -> bool {
    binary(token).is_some()
        || unary(token).is_some()
        || matches!(
            token,
            "unit"
                | "true"
                | "false"
                | "local"
                | "input"
                | "new"
                | "field"
                | "tag"
                | "wrap"
                | "is"
                | "payload"
                | "list"
                | "empty"
                | "last"
                | "at"
                | "minidx"
                | "call"
                | "m"
        )
}

struct Printer {
    text: String,
    limit: usize,
    unified: bool,
}
impl Printer {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            unified: false,
        }
    }
    fn token(&mut self, token: &str) -> Result<()> {
        let space = !self.text.is_empty() && !self.text.ends_with('\n');
        let length = self
            .text
            .len()
            .saturating_add(token.len())
            .saturating_add(usize::from(space));
        // The released FA1 byte-budget refusal keeps its diagnostic and
        // precedence. WK1 may impose an additional, smaller projection bound.
        bound(length)?;
        if length > self.limit {
            return Err(error("agent projection output byte limit"));
        }
        if space {
            self.text.push(' ');
        }
        self.text.push_str(token);
        Ok(())
    }
    fn line(&mut self) -> Result<()> {
        bound(self.text.len() + 1)?;
        if self.text.len() >= self.limit {
            return Err(error("agent projection output byte limit"));
        }
        self.text.push('\n');
        Ok(())
    }
    fn count(&mut self, count: usize) -> Result<()> {
        self.token(&count.to_string())
    }
    fn bindings(&mut self, bindings: &[Binding]) -> Result<()> {
        self.count(bindings.len())?;
        for binding in bindings {
            self.token(&binding.name)?;
            self.ty(&binding.ty)?;
        }
        Ok(())
    }
    fn ty(&mut self, ty: &Type) -> Result<()> {
        match ty {
            Type::Unit => self.token("unit"),
            Type::Bool => self.token("bool"),
            Type::I64 => self.token("int"),
            Type::U64 => self.token("uint"),
            Type::F32 => self.token("float"),
            Type::Record(name) => {
                self.token("record")?;
                self.token(name)
            }
            Type::Enum(name) => {
                self.token("enum")?;
                self.token(name)
            }
            Type::List(item) => {
                self.token("list")?;
                self.ty(item)
            }
        }
    }
    fn block(&mut self, body: &[Stmt]) -> Result<()> {
        self.count(body.len())?;
        for stmt in body {
            match stmt {
                Stmt::Set { local, value } => {
                    if self.unified
                        && let Expr::Binary { kind, left, right } = value
                        && let Expr::Local { name } = left.as_ref()
                        && name == local
                        && matches!(kind, Binary::Add | Binary::Sub)
                        && matches!(right.as_ref(), Expr::I64 { value: 1 })
                    {
                        self.token(if *kind == Binary::Add { "next" } else { "prev" })?;
                        self.token(local)?;
                        continue;
                    }
                    if self.unified
                        && let Expr::Binary { kind, left, right } = value
                        && let Expr::Local { name } = left.as_ref()
                        && name == local
                        && matches!(
                            kind,
                            Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                        )
                    {
                        self.token(match kind {
                            Binary::Add => "bump",
                            Binary::Sub => "sub",
                            Binary::Mul => "scale",
                            Binary::Div => "div",
                            Binary::Rem => "rem",
                            _ => unreachable!(),
                        })?;
                        self.token(local)?;
                        self.expr(right)?;
                        continue;
                    }
                    self.token("set")?;
                    self.token(local)?;
                    self.expr(value)?;
                }
                Stmt::SetField {
                    local,
                    field,
                    value,
                } => {
                    self.token("put")?;
                    self.token(local)?;
                    self.token(field)?;
                    self.expr(value)?;
                }
                Stmt::Push { local, value } => {
                    self.token("push")?;
                    self.token(local)?;
                    self.expr(value)?;
                }
                Stmt::Pop { local } => {
                    self.token("pop")?;
                    self.token(local)?;
                }
                Stmt::If {
                    test,
                    then,
                    otherwise,
                } => {
                    self.token("if")?;
                    self.expr(test)?;
                    self.block(then)?;
                    self.block(otherwise)?;
                }
                Stmt::While { test, body } => {
                    self.token("while")?;
                    self.expr(test)?;
                    self.block(body)?;
                }
                Stmt::Break => self.token("break")?,
                Stmt::Continue => self.token("continue")?,
                Stmt::Return { value } => {
                    self.token("ret")?;
                    self.expr(value)?;
                }
            }
        }
        Ok(())
    }
    fn expr(&mut self, value: &Expr) -> Result<()> {
        match value {
            Expr::Unit => self.token("unit")?,
            Expr::Bool { value } => self.token(if *value { "true" } else { "false" })?,
            Expr::I64 { value } => self.token(&value.to_string())?,
            Expr::U64 { value } => self.token(&format!("{value}u"))?,
            Expr::F32 { bits } => {
                if self.unified {
                    self.token(unified_float_literal(*bits)?.as_str())?;
                } else {
                    self.token(&format!("0x{bits:x}f"))?;
                }
            }
            Expr::Local { name } => {
                if reserved(name) || (self.unified && name == "host") {
                    self.token("local")?;
                }
                self.token(name)?;
            }
            Expr::Input { name } => {
                self.token("input")?;
                self.token(name)?;
            }
            Expr::Binary { kind, left, right } => {
                self.token(match kind {
                    Binary::Add => "add",
                    Binary::Sub => "sub",
                    Binary::Mul => "mul",
                    Binary::Div => "div",
                    Binary::Rem => "rem",
                    Binary::Eq => "eq",
                    Binary::Lt => "lt",
                    Binary::Le => "le",
                    Binary::And => "and",
                    Binary::Or => "or",
                })?;
                self.expr(left)?;
                self.expr(right)?;
            }
            Expr::Unary { kind, value } => {
                self.token(match kind {
                    Unary::Not => "not",
                    Unary::AbsF32 => "abs",
                    Unary::U64ToF32 => "tof",
                    Unary::I64ToF32 => "i64tof",
                    Unary::F32ToI64 => "ftoi64",
                    Unary::SqrtF32 => "sqrt",
                })?;
                self.expr(value)?;
            }
            Expr::Record { name, fields } => {
                self.token("new")?;
                self.token(name)?;
                self.expressions(fields)?;
            }
            Expr::Field { record, name } => {
                self.token("field")?;
                self.expr(record)?;
                self.token(name)?;
            }
            Expr::Variant {
                name,
                variant,
                payload,
            } => {
                self.token(if payload.is_some() { "wrap" } else { "tag" })?;
                self.token(name)?;
                self.token(variant)?;
                if let Some(payload) = payload {
                    self.expr(payload)?;
                }
            }
            Expr::IsVariant { value, variant } => {
                self.token("is")?;
                self.expr(value)?;
                self.token(variant)?;
            }
            Expr::Payload { value, variant } => {
                self.token("payload")?;
                self.expr(value)?;
                self.token(variant)?;
            }
            Expr::List { element, items } => {
                self.token("list")?;
                self.ty(element)?;
                self.expressions(items)?;
            }
            Expr::Empty { list } => {
                self.token("empty")?;
                self.expr(list)?;
            }
            Expr::Last { list } => {
                self.token("last")?;
                self.expr(list)?;
            }
            Expr::Index { list, index } => {
                self.token("at")?;
                self.expr(list)?;
                self.expr(index)?;
            }
            Expr::ArgMinF32 { list } => {
                self.token("minidx")?;
                self.expr(list)?;
            }
            Expr::Call { name, args } => {
                self.token("call")?;
                self.token(name)?;
                self.expressions(args)?;
            }
            Expr::HostCall {
                name,
                params,
                result,
                args,
            } => {
                self.token("host")?;
                self.token(name)?;
                self.ty(result)?;
                self.count(args.len())?;
                for (ty, arg) in params.iter().zip(args) {
                    self.ty(ty)?;
                    self.expr(arg)?;
                }
            }
            Expr::Method {
                receiver,
                name,
                args,
            } => {
                self.token("m")?;
                self.expr(receiver)?;
                self.token(name)?;
                self.expressions(args)?;
            }
        }
        Ok(())
    }
    fn expressions(&mut self, values: &[Expr]) -> Result<()> {
        self.count(values.len())?;
        for value in values {
            self.expr(value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_literal_buffer_refuses_before_writing_and_retains_complete_utf8() {
        use std::fmt::Write;
        let text = "é".repeat(32);
        let mut full = FloatLiteral::formatted(format_args!("{text}")).unwrap();
        assert_eq!(full.len, 64);
        assert!(full.write_str("f").is_err());
        assert_eq!(full.as_str(), text);
        assert_eq!(full.len, 64);
        let mut empty = FloatLiteral::formatted(format_args!("")).unwrap();
        assert!(empty.write_str(&"x".repeat(65)).is_err());
        assert_eq!(empty.as_str(), "");
        assert_eq!(empty.len, 0);
    }

    #[test]
    fn float_literal_buffer_fits_both_signs_and_every_exponent_boundary() {
        for sign in [0, 0x8000_0000] {
            for exponent in 0..=255_u32 {
                for mantissa in [0, 1, 0x003f_ffff, 0x007f_fffe, 0x007f_ffff] {
                    let bits = sign | (exponent << 23) | mantissa;
                    let value = f32::from_bits(bits);
                    let raw = FloatLiteral::formatted(format_args!("0x{bits:x}f")).unwrap();
                    assert!(raw.len <= 11);
                    if value.is_finite() {
                        // Both formats reserve a byte for the suffix. These
                        // boundaries include subnormals and the largest finite
                        // values; the public test separately checks exact text.
                        for mut literal in [
                            FloatLiteral::formatted(format_args!("{value}")).unwrap(),
                            FloatLiteral::formatted(format_args!("{value:e}")).unwrap(),
                        ] {
                            assert!(literal.len < 64);
                            literal.suffix().unwrap();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn printer_rejects_before_appending_over_budget_output() {
        let mut printer = Printer::new(256 * 1024);
        printer.token(&"x".repeat(256 * 1024)).unwrap();
        let before = printer.text.len();
        assert!(printer.token("overflow").is_err());
        assert!(printer.line().is_err());
        assert_eq!(printer.text.len(), before);
        let mut empty = Printer::new(256 * 1024);
        assert!(empty.token(&"x".repeat(256 * 1024 + 1)).is_err());
        assert!(empty.text.is_empty());
    }

    #[test]
    fn cumulative_node_budget_cannot_be_reset_by_another_declaration() {
        let text = format!(
            "fa1 v2 pub f 0 list int ret list int 32768 {}pub g 0 list int ret list int 32768 {}",
            "0 ".repeat(32768),
            "0 ".repeat(32768)
        );
        assert!(text.len() < 256 * 1024);
        assert!(
            Source::parse_agent(&text)
                .unwrap_err()
                .to_string()
                .contains("node count")
        );
    }

    #[test]
    fn unified_updates_charge_expanded_nodes_at_identical_depths() {
        for (short, long) in [
            ("bump x add 1 2", "set x add local x add 1 2"),
            ("scale add 3", "set add mul local add 3"),
            ("sub sub add 1 2", "set sub sub local sub add 1 2"),
            ("div div 3", "set div div local div 3"),
            ("rem rem 3", "set rem rem local rem 3"),
            ("next next", "set next add local next 1"),
            ("prev prev", "set prev sub local prev 1"),
        ] {
            for nodes in [
                0,
                MAX_NODES - 6,
                MAX_NODES - 5,
                MAX_NODES - 4,
                MAX_NODES - 1,
                MAX_NODES,
            ] {
                for depth in [0, MAX_DEPTH - 4, MAX_DEPTH - 3, MAX_DEPTH - 2, MAX_DEPTH] {
                    let parse = |text| {
                        let mut parser = Parser::new(text).unwrap();
                        parser.unified = true;
                        parser.nodes = nodes;
                        let result = parser.stmt(depth);
                        (format!("{result:?}"), parser.nodes)
                    };
                    assert_eq!(parse(short), parse(long), "nodes={nodes}, depth={depth}");
                }
            }
        }
    }

    #[test]
    fn unified_decimal_literals_keep_single_node_and_depth_admission() {
        for (short, raw) in [
            ("1f", "0x3f800000f"),
            ("-0f", "0x80000000f"),
            ("1e-45f", "0x1f"),
        ] {
            for nodes in [0, MAX_NODES - 1, MAX_NODES] {
                for depth in [0, MAX_DEPTH - 1, MAX_DEPTH, MAX_DEPTH + 1] {
                    let parse = |text| {
                        let mut parser = Parser::new(text).unwrap();
                        parser.unified = true;
                        parser.nodes = nodes;
                        let result = parser.expr(depth);
                        (format!("{result:?}"), parser.nodes)
                    };
                    assert_eq!(parse(short), parse(raw), "nodes={nodes}, depth={depth}");
                }
            }
        }
    }

    #[test]
    fn unified_checked_projection_obeys_its_bound_with_large_native_input() {
        let prefix = "fluxr 1\npub f 0 int ret ";
        let checked = format!("{prefix}{}1", " ".repeat(256 * 1024 - prefix.len() - 1));
        let document = parse_unified(&checked).unwrap();
        assert_eq!(document.checked_text, checked);
        assert!(
            parse_unified(&format!(
                "{prefix}{}1",
                " ".repeat(256 * 1024 - prefix.len())
            ))
            .unwrap_err()
            .to_string()
            .contains("byte budget")
        );
        // Native payloads do not consume the checked projection's byte budget.
        let native = " ".repeat(300 * 1024);
        let mixed = format!(
            "fluxr 1\nrust {}\n{native}\npub f 0 int ret 1",
            native.len()
        );
        assert_eq!(
            parse_unified(&mixed).unwrap().checked_text,
            "fluxr 1\npub f 0 int ret 1"
        );
    }
}
