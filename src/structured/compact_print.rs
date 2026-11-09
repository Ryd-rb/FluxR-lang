use super::*;
use std::fmt::Write;

impl Module {
    /// Emit compact source for a checked JSON, compact, or FSM module.
    /// This preserves the version and the canonical portable bytes.
    pub fn compact_text(&self) -> Result<String, Error> {
        let source = self.checked_source()?;
        let out = source.print_compact();
        let reparsed = Source::parse_compact(&out)?;
        let canonical = super::compile::encode_source(&reparsed)?;
        if canonical != self.canonical_source() {
            return Err(error("structured migration changed the canonical source"));
        }
        Ok(out)
    }
}

impl Source {
    /// Check the compact round trip directly when the compiler's input AST
    /// is still available. The AST retains numeric bits and declaration order.
    pub(crate) fn checked_compact_text(&self) -> Result<String, Error> {
        let out = self.print_compact();
        if Self::parse_compact(&out)? != *self {
            return Err(error("structured migration changed the canonical source"));
        }
        Ok(out)
    }

    fn print_compact(&self) -> String {
        let mut out = format!("v{}", self.version);
        if !self.inputs.is_empty() {
            out.push_str("\n(inputs");
            bindings(&mut out, &self.inputs);
            out.push(')');
        }
        for record in &self.records {
            write!(out, "\n(rec {}", record.name).unwrap();
            bindings(&mut out, &record.fields);
            out.push(')');
        }
        for enumeration in &self.enums {
            write!(out, "\n(enum {}", enumeration.name).unwrap();
            for variant in &enumeration.variants {
                write!(out, " {}", variant.name).unwrap();
                if let Some(payload) = &variant.payload {
                    out.push(':');
                    ty(&mut out, payload);
                }
            }
            out.push(')');
        }
        for function in &self.functions {
            write!(out, "\n({} ", if function.export { "pub" } else { "fn" }).unwrap();
            if let Some(receiver) = &function.receiver {
                write!(out, "{receiver}.").unwrap();
            }
            write!(out, "{} (", function.name).unwrap();
            binding_group(&mut out, &function.params);
            out.push_str(") ");
            ty(&mut out, &function.result);
            out.push_str(" (");
            binding_group(&mut out, &function.locals);
            out.push(')');
            statements(&mut out, &function.body);
            out.push(')');
        }
        out
    }
}

fn bindings(out: &mut String, declarations: &[Binding]) {
    for binding in declarations {
        out.push(' ');
        write_binding(out, binding);
    }
}

fn binding_group(out: &mut String, declarations: &[Binding]) {
    if let Some((first, rest)) = declarations.split_first() {
        write_binding(out, first);
        bindings(out, rest);
    }
}

fn write_binding(out: &mut String, binding: &Binding) {
    write!(out, "{}:", binding.name).unwrap();
    ty(out, &binding.ty);
}

fn reserved_type(name: &str) -> bool {
    matches!(name, "unit" | "bool" | "i64" | "u64" | "f32")
}

fn ty(out: &mut String, kind: &Type) {
    match kind {
        Type::Unit => out.push_str("unit"),
        Type::Bool => out.push_str("bool"),
        Type::I64 => out.push_str("i64"),
        Type::U64 => out.push_str("u64"),
        Type::F32 => out.push_str("f32"),
        Type::Record(name) => {
            if reserved_type(name) {
                out.push('@');
            }
            out.push_str(name);
        }
        Type::Enum(name) => write!(out, "#{name}").unwrap(),
        Type::List(item) => {
            out.push('[');
            ty(out, item);
            out.push(']');
        }
    }
}

fn statements(out: &mut String, body: &[Stmt]) {
    for statement in body {
        out.push_str(" (");
        match statement {
            Stmt::Set { local, value } => {
                write!(out, "= {local}").unwrap();
                expr(out, value);
            }
            Stmt::SetField {
                local,
                field,
                value,
            } => {
                write!(out, "= {local}.{field}").unwrap();
                expr(out, value);
            }
            Stmt::Push { local, value } => {
                write!(out, "push {local}").unwrap();
                expr(out, value);
            }
            Stmt::Pop { local } => write!(out, "pop {local}").unwrap(),
            Stmt::If {
                test,
                then,
                otherwise,
            } => {
                out.push_str("if");
                expr(out, test);
                block(out, then);
                if !otherwise.is_empty() {
                    block(out, otherwise);
                }
            }
            Stmt::While { test, body } => {
                out.push_str("while");
                expr(out, test);
                block(out, body);
            }
            Stmt::Break => out.push_str("break"),
            Stmt::Continue => out.push_str("continue"),
            Stmt::Return { value } => {
                out.push_str("ret");
                expr(out, value);
            }
        }
        out.push(')');
    }
}

fn block(out: &mut String, body: &[Stmt]) {
    out.push('(');
    statements(out, body);
    out.push(')');
}

fn expr(out: &mut String, value: &Expr) {
    use Expr::*;
    match value {
        Unit => out.push_str(" unit"),
        Bool { value } => write!(out, " {value}").unwrap(),
        I64 { value } => write!(out, " {value}").unwrap(),
        U64 { value } => write!(out, " {value}u").unwrap(),
        F32 { bits } => write!(out, " 0x{bits:x}f").unwrap(),
        Local { name } if matches!(name.as_str(), "unit" | "true" | "false") => {
            write!(out, "(local {name})").unwrap();
        }
        Local { name } => write!(out, " {name}").unwrap(),
        Input { name } => write!(out, " ${name}").unwrap(),
        Field { record, name } if matches!(record.as_ref(), Local { .. }) => {
            let Local { name: local } = record.as_ref() else {
                unreachable!()
            };
            write!(out, " {local}.{name}").unwrap();
        }
        compound => {
            out.push('(');
            match compound {
                Binary { kind, left, right } => {
                    out.push_str(match kind {
                        super::Binary::Add => "+",
                        super::Binary::Sub => "-",
                        super::Binary::Mul => "*",
                        super::Binary::Div => "/",
                        super::Binary::Rem => "%",
                        super::Binary::Eq => "==",
                        super::Binary::Lt => "<",
                        super::Binary::Le => "<=",
                        super::Binary::And => "&&",
                        super::Binary::Or => "||",
                    });
                    expr(out, left);
                    expr(out, right);
                }
                Unary { kind, value } => {
                    out.push_str(match kind {
                        super::Unary::Not => "!",
                        super::Unary::AbsF32 => "abs",
                        super::Unary::U64ToF32 => "tof",
                    });
                    expr(out, value);
                }
                Record { name, fields } => {
                    write!(out, "new {name}").unwrap();
                    expressions(out, fields);
                }
                Field { record, name } => {
                    out.push_str("field");
                    expr(out, record);
                    write!(out, " {name}").unwrap();
                }
                Variant {
                    name,
                    variant,
                    payload,
                } => {
                    write!(out, "tag {name}.{variant}").unwrap();
                    if let Some(payload) = payload {
                        expr(out, payload);
                    }
                }
                IsVariant { value, variant } | Payload { value, variant } => {
                    out.push_str(if matches!(compound, IsVariant { .. }) {
                        "is"
                    } else {
                        "payload"
                    });
                    expr(out, value);
                    write!(out, " {variant}").unwrap();
                }
                List { element, items } => {
                    out.push_str("list ");
                    ty(out, element);
                    expressions(out, items);
                }
                Empty { list } | Last { list } | ArgMinF32 { list } => {
                    out.push_str(match compound {
                        Empty { .. } => "empty",
                        Last { .. } => "last",
                        _ => "minidx",
                    });
                    expr(out, list);
                }
                Index { list, index } => {
                    out.push_str("at");
                    expr(out, list);
                    expr(out, index);
                }
                Call { name, args } => {
                    write!(out, "call {name}").unwrap();
                    expressions(out, args);
                }
                Method {
                    receiver,
                    name,
                    args,
                } => {
                    out.push('m');
                    expr(out, receiver);
                    write!(out, " {name}").unwrap();
                    expressions(out, args);
                }
                _ => unreachable!("atoms emitted before compound forms"),
            }
            out.push(')');
        }
    }
}

fn expressions(out: &mut String, values: &[Expr]) {
    for value in values {
        expr(out, value);
    }
}
