//! Short-circuit boolean policies over host-supplied facts.

use crate::structured::{Binary, Expr, Unary, parse_expression};
use crate::{Error, bound};
use std::fmt::Write;

#[derive(Debug)]
enum Node {
    Fact(usize),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
}

impl Node {
    fn parse(expr: Expr, seen: &mut u16) -> Result<Self, Error> {
        match expr {
            Expr::Local { name } => {
                let [letter] = name.as_bytes() else {
                    return Err(Error("gate facts use a through p".into()));
                };
                if !matches!(letter, b'a'..=b'p') {
                    return Err(Error("gate facts use a through p".into()));
                }
                let index = usize::from(letter - b'a');
                let bit = 1_u16 << index;
                if *seen & bit != 0 {
                    return Err(Error("gate facts may be evaluated at most once".into()));
                }
                *seen |= bit;
                Ok(Self::Fact(index))
            }
            Expr::Unary {
                kind: Unary::Not,
                value,
            } => Ok(Self::Not(Box::new(Self::parse(*value, seen)?))),
            Expr::Binary {
                kind: kind @ (Binary::And | Binary::Or),
                left,
                right,
            } => {
                let left = Box::new(Self::parse(*left, seen)?);
                let right = Box::new(Self::parse(*right, seen)?);
                Ok(match kind {
                    Binary::And => Self::And(left, right),
                    _ => Self::Or(left, right),
                })
            }
            _ => Err(Error(
                "gate expects boolean facts, negation, conjunction or disjunction".into(),
            )),
        }
    }

    fn evaluate(&self, fact: &mut impl FnMut(usize) -> bool) -> bool {
        match self {
            Self::Fact(index) => fact(*index),
            Self::Not(value) => !value.evaluate(fact),
            Self::And(left, right) => left.evaluate(fact) && right.evaluate(fact),
            Self::Or(left, right) => left.evaluate(fact) || right.evaluate(fact),
        }
    }

    fn emit(&self, output: &mut String, parent: u8) {
        let precedence = match self {
            Self::Or(..) => 1,
            Self::And(..) => 2,
            Self::Not(..) => 3,
            Self::Fact(..) => 4,
        };
        if precedence < parent {
            output.push('(');
        }
        match self {
            Self::Fact(index) => write!(output, "p{index}()").unwrap(),
            Self::Not(value) => {
                output.push('!');
                value.emit(output, precedence);
            }
            Self::And(left, right) | Self::Or(left, right) => {
                left.emit(output, precedence);
                output.push_str(if precedence == 2 { " && " } else { " || " });
                right.emit(output, precedence);
            }
        }
        if precedence < parent {
            output.push(')');
        }
    }
}

/// A gate evaluates ordered, lazy boolean facts. Lowering to Rust preserves
/// short circuit and lets the host keep expensive observations as closures.
#[derive(Debug)]
pub struct Gate {
    tree: Node,
    inputs: usize,
}

impl Gate {
    pub fn compile_text(source: &str) -> Result<Self, Error> {
        bound(source.len())?;
        let text = source.trim();
        let expanded;
        let expression = if let Some((operator, names)) = flat_gate(text) {
            expanded = format!(
                "({operator} {})",
                names
                    .chars()
                    .map(|letter| letter.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            expanded.as_str()
        } else {
            text
        };
        let mut seen = 0;
        let tree = Node::parse(parse_expression(expression)?, &mut seen)?;
        let inputs = seen.count_ones() as usize;
        if inputs == 0 || u32::from(seen) != (1_u32 << inputs) - 1 {
            return Err(Error("gate facts must be consecutive from a".into()));
        }
        Ok(Self { tree, inputs })
    }

    pub fn evaluate(&self, mut fact: impl FnMut(usize) -> bool) -> bool {
        self.tree.evaluate(&mut fact)
    }

    pub fn rust_function(&self, name: &str) -> Result<String, Error> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(Error("invalid generated gate name".into()));
        }
        let mut source = format!("#[inline(always)]\npub(crate) fn fluxr_{name}(");
        for index in 0..self.inputs {
            write!(source, "p{index}: impl FnOnce() -> bool, ").unwrap();
        }
        source.push_str(") -> bool {\n    ");
        self.tree.emit(&mut source, 0);
        source.push_str("\n}\n");
        Ok(source)
    }
}

fn flat_gate(text: &str) -> Option<(char, &str)> {
    let first = text.chars().next()?;
    let names = &text[first.len_utf8()..];
    (matches!(first, '&' | '|')
        && (2..=16).contains(&names.len())
        && names.bytes().all(|byte| matches!(byte, b'a'..=b'p')))
    .then_some((first, names))
}
