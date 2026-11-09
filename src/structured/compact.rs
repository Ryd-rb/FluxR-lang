use super::{
    Binary, Binding, Enum, Expr, Function, Record, Source, Stmt, Type, Unary, Variant, error,
};
use crate::{Error, bound};

type Result<T> = std::result::Result<T, Error>;

impl Source {
    pub fn parse_compact(text: &str) -> Result<Self> {
        bound(text.len())?;
        let mut p = Parser::new(text);
        let version = p
            .word()?
            .strip_prefix('v')
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| error("compact module requires v1 or v2"))?;
        let mut source = Self {
            version,
            inputs: vec![],
            records: vec![],
            enums: vec![],
            functions: vec![],
        };
        while !p.rest.trim_start().is_empty() {
            p.expect("(")?;
            match p.word()? {
                "inputs" => source.inputs.extend(p.items(binding)?),
                "rec" => source.records.push(Record {
                    name: p.word()?.into(),
                    fields: p.items(binding)?,
                }),
                "enum" => source.enums.push(Enum {
                    name: p.word()?.into(),
                    variants: p.items(variant)?,
                }),
                kind @ ("fn" | "pub") => {
                    let name = p.word()?;
                    let (receiver, name) = name
                        .split_once('.')
                        .map_or((None, name), |(ty, method)| (Some(ty.into()), method));
                    let params = p.group(binding)?;
                    let result = ty(p.word()?)?;
                    let (locals, body) = if p.rest.trim_start().starts_with('=') {
                        p.expect("=")?;
                        let tail = p.tail()?;
                        p.expect(")")?;
                        (vec![], vec![tail])
                    } else {
                        (p.group(binding)?, p.statements()?)
                    };
                    source.functions.push(Function {
                        name: name.into(),
                        receiver,
                        export: kind == "pub",
                        params,
                        locals,
                        result,
                        body,
                    });
                }
                tag => return Err(error(format!("unknown compact declaration {tag}"))),
            }
        }
        Ok(source)
    }
}

impl Stmt {
    /// Parse a compact edit body containing consecutive parenthesized statements.
    pub fn parse_compact_body(text: &str) -> Result<Vec<Self>> {
        bound(text.len())?;
        let mut p = Parser::new(text);
        let mut body = Vec::new();
        while !p.rest.trim_start().is_empty() {
            body.extend(p.stmt()?);
        }
        Ok(body)
    }
}

fn ty(word: &str) -> Result<Type> {
    let mut word = word;
    let mut lists = 0;
    while let Some(inner) = word.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        lists += 1;
        if lists > 32 {
            return Err(error("compact type nesting exceeds limit"));
        }
        word = inner;
    }
    let mut result = match word {
        "unit" => Type::Unit,
        "bool" => Type::Bool,
        "i64" => Type::I64,
        "u64" => Type::U64,
        "f32" => Type::F32,
        _ if word.starts_with('#') => Type::Enum(word[1..].into()),
        _ if word.starts_with('@') => Type::Record(word[1..].into()),
        _ => Type::Record(word.into()),
    };
    for _ in 0..lists {
        result = Type::List(Box::new(result));
    }
    Ok(result)
}

fn binding(p: &mut Parser<'_>) -> Result<Binding> {
    let (name, kind) = p
        .word()?
        .split_once(':')
        .ok_or_else(|| error("binding requires name:type"))?;
    Ok(Binding {
        name: name.into(),
        ty: ty(kind)?,
    })
}

fn variant(p: &mut Parser<'_>) -> Result<Variant> {
    let word = p.word()?;
    if let Some((name, kind)) = word.split_once(':') {
        return Ok(Variant {
            name: name.into(),
            payload: Some(ty(kind)?),
        });
    }
    Ok(Variant {
        name: word.into(),
        payload: None,
    })
}

#[derive(Clone, Copy)]
struct Parser<'a> {
    rest: &'a str,
    depth: u8,
}

impl<'a> Parser<'a> {
    fn new(rest: &'a str) -> Self {
        Self { rest, depth: 0 }
    }

    fn next(&mut self) -> Result<&'a str> {
        self.rest = self.rest.trim_start();
        let end = match self.rest.as_bytes().first() {
            None => return Err(error("unexpected end of compact source")),
            Some(b'(') => {
                if self.depth == 96 {
                    return Err(error("compact nesting exceeds limit"));
                }
                self.depth += 1;
                1
            }
            Some(b')') => {
                self.depth = self
                    .depth
                    .checked_sub(1)
                    .ok_or_else(|| error("unexpected closing parenthesis"))?;
                1
            }
            _ => self
                .rest
                .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
                .unwrap_or(self.rest.len()),
        };
        let (token, rest) = self.rest.split_at(end);
        self.rest = rest;
        Ok(token)
    }

    fn word(&mut self) -> Result<&'a str> {
        let word = self.next()?;
        if matches!(word, "(" | ")") {
            return Err(error("expected compact identifier or literal"));
        }
        Ok(word)
    }

    fn expect(&mut self, expected: &str) -> Result<()> {
        if self.next()? != expected {
            return Err(error(format!("expected {expected} in compact source")));
        }
        Ok(())
    }

    fn items<T>(&mut self, parse: impl Fn(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let mut items = Vec::new();
        while !self.rest.trim_start().starts_with(')') {
            items.push(parse(self)?);
        }
        self.expect(")")?;
        Ok(items)
    }

    fn group<T>(&mut self, parse: impl Fn(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        self.expect("(")?;
        self.items(parse)
    }

    fn statements(&mut self) -> Result<Vec<Stmt>> {
        let mut body = Vec::new();
        while !self.rest.trim_start().starts_with(')') {
            body.extend(self.stmt()?);
        }
        self.expect(")")?;
        Ok(body)
    }

    fn block(&mut self) -> Result<Vec<Stmt>> {
        self.expect("(")?;
        self.statements()
    }

    fn stmt(&mut self) -> Result<Vec<Stmt>> {
        self.expect("(")?;
        let item = match self.word()? {
            "=" => {
                let local = self.word()?;
                let value = self.expr()?;
                match local.split_once('.') {
                    Some((local, field)) => Stmt::SetField {
                        local: local.into(),
                        field: field.into(),
                        value,
                    },
                    None => Stmt::Set {
                        local: local.into(),
                        value,
                    },
                }
            }
            "push" => Stmt::Push {
                local: self.word()?.into(),
                value: self.expr()?,
            },
            "pop" => Stmt::Pop {
                local: self.word()?.into(),
            },
            "if" => {
                let test = self.expr()?;
                let then = self.block()?;
                let otherwise = if self.rest.trim_start().starts_with('(') {
                    self.block()?
                } else {
                    vec![]
                };
                Stmt::If {
                    test,
                    then,
                    otherwise,
                }
            }
            "when" => Stmt::If {
                test: self.expr()?,
                then: self.stmt()?,
                otherwise: vec![],
            },
            "find" => {
                let index = self.word()?.to_owned();
                let end = self.expr()?;
                let row = self.word()?.to_owned();
                if row == index {
                    return Err(error("find row and index must differ"));
                }
                let rows = self.boxed()?;
                let test = self.expr()?;
                self.expect(")")?;
                let scan = scan(
                    &index,
                    end.clone(),
                    vec![
                        Stmt::Set {
                            local: row,
                            value: Expr::Index {
                                list: rows,
                                index: Box::new(local(&index)),
                            },
                        },
                        Stmt::If {
                            test,
                            then: vec![Stmt::Return {
                                value: local(&index),
                            }],
                            otherwise: vec![],
                        },
                    ],
                );
                let [init, step] = scan;
                return Ok(vec![init, step, Stmt::Return { value: end }]);
            }
            "indices" => {
                let output = self.word()?.to_owned();
                let index = self.word()?.to_owned();
                if output == index {
                    return Err(error("indices output and index must differ"));
                }
                let end = self.expr()?;
                let test = self.expr()?;
                self.expect(")")?;
                let [init, step] = scan(
                    &index,
                    end,
                    vec![Stmt::If {
                        test,
                        then: vec![Stmt::Push {
                            local: output.clone(),
                            value: local(&index),
                        }],
                        otherwise: vec![],
                    }],
                );
                return Ok(vec![
                    Stmt::Set {
                        local: output.clone(),
                        value: Expr::List {
                            element: Type::U64,
                            items: vec![],
                        },
                    },
                    init,
                    step,
                    Stmt::Return {
                        value: local(&output),
                    },
                ]);
            }
            "while" => Stmt::While {
                test: self.expr()?,
                body: self.block()?,
            },
            "break" => Stmt::Break,
            "continue" => Stmt::Continue,
            "ret" => Stmt::Return {
                value: self.expr()?,
            },
            "?" => self.selection()?,
            tag => return Err(error(format!("unknown compact statement {tag}"))),
        };
        self.expect(")")?;
        Ok(vec![item])
    }

    fn tail(&mut self) -> Result<Stmt> {
        let mut lookahead = *self;
        if matches!(lookahead.next(), Ok("(")) && matches!(lookahead.next(), Ok("?")) {
            *self = lookahead;
            let selection = self.selection()?;
            self.expect(")")?;
            Ok(selection)
        } else {
            Ok(Stmt::Return {
                value: self.expr()?,
            })
        }
    }

    fn selection(&mut self) -> Result<Stmt> {
        Ok(Stmt::If {
            test: self.expr()?,
            then: vec![self.tail()?],
            otherwise: vec![self.tail()?],
        })
    }

    fn boxed(&mut self) -> Result<Box<Expr>> {
        self.expr().map(Box::new)
    }

    fn expr(&mut self) -> Result<Expr> {
        let token = self.next()?;
        if token != "(" {
            return atom(token);
        }
        let tag = self.word()?;
        if matches!(tag, "&" | "|") {
            let kind = if tag == "&" { Binary::And } else { Binary::Or };
            let mut terms = self.items(Self::expr)?.into_iter();
            let left = terms
                .next()
                .ok_or_else(|| error("empty compact boolean chain"))?;
            if terms.len() == 0 {
                return Err(error("compact boolean chain needs at least two terms"));
            }
            // The structured checker limits expression depth. Bound the chain
            // before constructing an increasingly deep boxed expression.
            if terms.len() > 31 {
                return Err(error("compact boolean chain exceeds limit"));
            }
            return Ok(terms.fold(left, |left, right| combine(kind, left, right)));
        }
        if let Some(kind) = binary(tag) {
            let left = self.boxed()?;
            let right = self.boxed()?;
            self.expect(")")?;
            return Ok(Expr::Binary { kind, left, right });
        }
        let item = match tag {
            "!" | "abs" | "tof" | "i64tof" | "ftoi64" | "sqrt" => {
                let kind = match tag {
                    "!" => Unary::Not,
                    "abs" => Unary::AbsF32,
                    "i64tof" => Unary::I64ToF32,
                    "ftoi64" => Unary::F32ToI64,
                    "sqrt" => Unary::SqrtF32,
                    _ => Unary::U64ToF32,
                };
                Expr::Unary {
                    kind,
                    value: self.boxed()?,
                }
            }
            "new" => {
                return Ok(Expr::Record {
                    name: self.word()?.into(),
                    fields: self.items(Self::expr)?,
                });
            }
            "list" => {
                return Ok(Expr::List {
                    element: ty(self.word()?)?,
                    items: self.items(Self::expr)?,
                });
            }
            "local" => Expr::Local {
                name: self.word()?.into(),
            },
            "input" => Expr::Input {
                name: self.word()?.into(),
            },
            "field" => Expr::Field {
                record: self.boxed()?,
                name: self.word()?.into(),
            },
            "at" => Expr::Index {
                list: self.boxed()?,
                index: self.boxed()?,
            },
            "empty" => Expr::Empty {
                list: self.boxed()?,
            },
            "last" => Expr::Last {
                list: self.boxed()?,
            },
            "minidx" => Expr::ArgMinF32 {
                list: self.boxed()?,
            },
            "tag" => {
                let (name, variant) = self
                    .word()?
                    .split_once('.')
                    .ok_or_else(|| error("tag requires Enum.variant"))?;
                let payload = if self.rest.trim_start().starts_with(')') {
                    None
                } else {
                    Some(self.boxed()?)
                };
                Expr::Variant {
                    name: name.into(),
                    variant: variant.into(),
                    payload,
                }
            }
            "is" => Expr::IsVariant {
                value: self.boxed()?,
                variant: self.word()?.into(),
            },
            "payload" => Expr::Payload {
                value: self.boxed()?,
                variant: self.word()?.into(),
            },
            "m" => {
                return Ok(Expr::Method {
                    receiver: self.boxed()?,
                    name: self.word()?.into(),
                    args: self.items(Self::expr)?,
                });
            }
            "call" => {
                return Ok(Expr::Call {
                    name: self.word()?.into(),
                    args: self.items(Self::expr)?,
                });
            }
            method if method.contains('.') => {
                let (receiver, name) = method.rsplit_once('.').expect("method separator");
                return Ok(Expr::Method {
                    receiver: Box::new(atom(receiver)?),
                    name: name.into(),
                    args: self.items(Self::expr)?,
                });
            }
            name => {
                return Ok(Expr::Call {
                    name: name.into(),
                    args: self.items(Self::expr)?,
                });
            }
        };
        self.expect(")")?;
        Ok(item)
    }
}

pub(crate) fn parse_expression(text: &str) -> Result<Expr> {
    bound(text.len())?;
    let mut parser = Parser::new(text);
    let result = parser.expr()?;
    if !parser.rest.trim().is_empty() {
        return Err(error("unexpected compact expression suffix"));
    }
    Ok(result)
}

fn local(name: &str) -> Expr {
    Expr::Local { name: name.into() }
}

fn combine(kind: Binary, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        kind,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn scan(index: &str, end: Expr, mut body: Vec<Stmt>) -> [Stmt; 2] {
    body.push(Stmt::Set {
        local: index.into(),
        value: combine(Binary::Add, local(index), Expr::U64 { value: 1 }),
    });
    [
        Stmt::Set {
            local: index.into(),
            value: Expr::U64 { value: 0 },
        },
        Stmt::While {
            test: combine(Binary::Lt, local(index), end),
            body,
        },
    ]
}

fn atom(token: &str) -> Result<Expr> {
    if matches!(token, "(" | ")") {
        return Err(error("expected expression"));
    }
    if token
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_digit() || *b == b'-')
    {
        if let Some(s) = token.strip_suffix('u') {
            return Ok(Expr::U64 {
                value: s.parse().map_err(|_| error("invalid u64 literal"))?,
            });
        }
        if let Some(s) = token.strip_suffix('f') {
            let bits = if let Some(hex) = s.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).map_err(|_| error("invalid f32 bits"))?
            } else {
                s.parse::<f32>()
                    .map_err(|_| error("invalid f32 literal"))?
                    .to_bits()
            };
            return Ok(Expr::F32 { bits });
        }
        return Ok(Expr::I64 {
            value: token.parse().map_err(|_| error("invalid i64 literal"))?,
        });
    }
    Ok(match token {
        "unit" => Expr::Unit,
        "true" => Expr::Bool { value: true },
        "false" => Expr::Bool { value: false },
        _ if token.starts_with('$') => Expr::Input {
            name: token[1..].into(),
        },
        _ => {
            let mut parts = token.split('.');
            let mut value = Expr::Local {
                name: parts.next().unwrap().into(),
            };
            for (index, name) in parts.enumerate() {
                if index == 32 {
                    return Err(error("compact field chain exceeds limit"));
                }
                value = Expr::Field {
                    record: Box::new(value),
                    name: name.into(),
                };
            }
            value
        }
    })
}

fn binary(tag: &str) -> Option<Binary> {
    Some(match tag {
        "+" => Binary::Add,
        "-" => Binary::Sub,
        "*" => Binary::Mul,
        "/" => Binary::Div,
        "%" => Binary::Rem,
        "==" => Binary::Eq,
        "<" => Binary::Lt,
        "<=" => Binary::Le,
        "&&" => Binary::And,
        "||" => Binary::Or,
        _ => return None,
    })
}
