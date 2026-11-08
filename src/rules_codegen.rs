use super::{Binary, CANONICAL_F32_NAN, Error, Op, RuleProgram};
use std::fmt::Write;

impl RuleProgram {
    /// Lower a validated rule to a host-inlinable `fluxr_<name>` Rust function.
    /// Its i64 parameters follow input order. `None` is an arithmetic error.
    pub fn rust_function(&self, name: &str) -> Result<String, Error> {
        self.emit_rust(name, false)
    }

    /// Lower a rule with its ordered inputs in one fixed-size i64 array.
    pub fn rust_array_function(&self, name: &str) -> Result<String, Error> {
        self.emit_rust(name, true)
    }

    fn emit_rust(&self, name: &str, array_input: bool) -> Result<String, Error> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(Error("invalid generated rule name".into()));
        }
        let mut output = format!("#[inline(always)]\npub(crate) fn fluxr_{name}(");
        if array_input {
            output.push('[');
        }
        for input in 0..self.inputs {
            write!(output, "_i{input}").unwrap();
            if !array_input {
                output.push_str(": i64");
            }
            output.push_str(", ");
        }
        if array_input {
            write!(output, "]: [i64; {}]", self.inputs).unwrap();
        }
        output.push_str(") -> Option<i64> {\n");
        let mut stack = Vec::new();
        for (index, &op) in self.ops.iter().enumerate() {
            if op == Op::Guard {
                let condition: Value = stack.pop().expect("validated stack");
                writeln!(
                    output,
                    "    if {} == 0 {{ return Some(0); }}",
                    condition.integer()
                )
                .unwrap();
                continue;
            }
            write!(output, "    let v{index} = ").unwrap();
            let mut binary32 = false;
            match op {
                Op::Input(input) => write!(output, "_i{input}").unwrap(),
                Op::Const(value) => write!(output, "{value}_i64").unwrap(),
                Op::Binary(operation) => {
                    let right = stack.pop().expect("validated stack");
                    let left = stack.pop().expect("validated stack");
                    binary32 = operation.write_rust(&mut output, left, right);
                }
                Op::Select => {
                    let otherwise = stack.pop().expect("validated stack");
                    let then = stack.pop().expect("validated stack");
                    let condition = stack.pop().expect("validated stack");
                    write!(
                        output,
                        "if {} != 0 {{ {} }} else {{ {} }}",
                        condition.integer(),
                        then.integer(),
                        otherwise.integer(),
                    )
                    .unwrap();
                }
                Op::Guard => unreachable!(),
            }
            output.push_str(";\n");
            stack.push(Value { index, binary32 });
        }
        writeln!(output, "    Some({})\n}}", stack[0].integer()).unwrap();
        Ok(output)
    }
}

#[derive(Clone, Copy)]
struct Value {
    index: usize,
    binary32: bool,
}

impl Value {
    // Float consumers cannot observe NaN payloads. Materialize canonical bits
    // only when an integer operation or the return boundary can observe them.
    fn integer(self) -> String {
        let index = self.index;
        if self.binary32 {
            format!(
                "i64::from(if v{index}.is_nan() {{ {CANONICAL_F32_NAN}_u32 }} else {{ v{index}.to_bits() }})"
            )
        } else {
            format!("v{index}")
        }
    }

    fn float(self) -> String {
        let index = self.index;
        if self.binary32 {
            format!("v{index}")
        } else {
            format!("f32::from_bits(v{index} as u32)")
        }
    }
}

impl Binary {
    fn write_rust(self, output: &mut String, left: Value, right: Value) -> bool {
        if matches!(self, Self::F32Lt | Self::F32Le) {
            let compare = &self.token()[1..];
            write!(
                output,
                "i64::from({} {compare} {})",
                left.float(),
                right.float()
            )
            .unwrap();
            return false;
        }
        if matches!(
            self,
            Self::F32Add | Self::F32Sub | Self::F32Mul | Self::F32Div
        ) {
            let operator = &self.token()[1..];
            write!(output, "{} {operator} {}", left.float(), right.float()).unwrap();
            return true;
        }
        let left = left.integer();
        let right = right.integer();
        let method = match self {
            Self::Add => "checked_add",
            Self::Sub => "checked_sub",
            Self::Mul => "checked_mul",
            Self::Div => "checked_div",
            Self::Rem => "checked_rem",
            Self::Min => "min",
            Self::Max => "max",
            Self::Lt => {
                write!(output, "i64::from({left} < {right})").unwrap();
                return false;
            }
            Self::Eq => {
                write!(output, "i64::from({left} == {right})").unwrap();
                return false;
            }
            Self::BitAnd => {
                write!(output, "{left} & {right}").unwrap();
                return false;
            }
            Self::F32Lt
            | Self::F32Le
            | Self::F32Add
            | Self::F32Sub
            | Self::F32Mul
            | Self::F32Div => {
                unreachable!()
            }
        };
        write!(output, "{left}.{method}({right})").unwrap();
        if !matches!(self, Self::Min | Self::Max) {
            output.push('?');
        }
        false
    }
}
