//! Executable specification for the bounded v2 list-effect fragment.
//! It consumes host values, never compiled instructions or runtime cost metadata.
use super::{CELLS, FRAMES, FUEL};
use fluxr::structured::{Limits, Value};

const ARITY: &str = "host argument or input count differs from interface";
const INDEX: &str = "list index out of bounds";

#[derive(Clone, Copy)]
pub(super) enum Expr {
    Input,
    Argument,
    Uninitialized,
    Literal(&'static [u32]),
    Scan(&'static Expr),
    At(&'static Expr, &'static Expr),
    Zero,
}

pub(super) struct Spec {
    // Each root is an f32 nested under this many list constructors.
    pub inputs: &'static [u8],
    pub arguments: &'static [u8],
    pub locals: usize,
    pub body: Expr,
}

impl Spec {
    pub(super) fn run(
        &self,
        inputs: &[Value],
        arguments: &[Value],
        limits: Limits,
    ) -> Result<Value, &'static str> {
        if inputs.len() != self.inputs.len() || arguments.len() != self.arguments.len() {
            return Err(ARITY);
        }
        let mut budget = Budget {
            limits,
            fuel: 0,
            cells: 0,
        };
        for (value, &lists) in inputs
            .iter()
            .zip(self.inputs)
            .chain(arguments.iter().zip(self.arguments))
        {
            budget.admit(value, lists)?;
        }
        if limits.frames == 0 {
            return Err(FRAMES);
        }
        budget.cells(1 + arguments.len() + self.locals)?;
        let value = budget.eval(self.body, inputs, arguments)?;
        budget.fuel(1)?; // Return.
        budget.cells(1)?;
        Ok(value)
    }
}

// Record cumulative usage in the bounded fragment. In particular, an
// unaffordable child is rejected before its value tag is examined.
struct Budget {
    limits: Limits,
    fuel: u64,
    cells: usize,
}

impl Budget {
    fn fuel(&mut self, count: u64) -> Result<(), &'static str> {
        self.fuel += count;
        if self.fuel > self.limits.instructions {
            Err(FUEL)
        } else {
            Ok(())
        }
    }

    fn cells(&mut self, count: usize) -> Result<(), &'static str> {
        self.cells += count;
        if self.cells > self.limits.cells {
            Err(CELLS)
        } else {
            Ok(())
        }
    }

    fn admit(&mut self, value: &Value, lists: u8) -> Result<(), &'static str> {
        self.cells(1)?;
        match (lists, value) {
            (0, Value::F32(_)) => Ok(()),
            (0, _) => Err("host value differs from F32"),
            (_, Value::List(children)) => children
                .iter()
                .try_for_each(|child| self.admit(child, lists - 1)),
            (1, _) => Err("host value differs from List(F32)"),
            (2, _) => Err("host value differs from List(List(F32))"),
            _ => unreachable!("spec only models up to two list layers"),
        }
    }

    fn eval(
        &mut self,
        expr: Expr,
        inputs: &[Value],
        arguments: &[Value],
    ) -> Result<Value, &'static str> {
        match expr {
            Expr::Input | Expr::Argument | Expr::Uninitialized | Expr::Zero => {
                self.fuel(1)?;
                match expr {
                    Expr::Input => Ok(inputs[0].clone()),
                    Expr::Argument => Ok(arguments[0].clone()),
                    Expr::Uninitialized => Err("read of uninitialized local"),
                    Expr::Zero => Ok(Value::U64(0)),
                    _ => unreachable!(),
                }
            }
            Expr::Literal(bits) => {
                self.fuel(bits.len() as u64 + 1)?;
                self.cells(bits.len() + 1)?;
                Ok(Value::List(
                    bits.iter()
                        .copied()
                        .map(Value::F32)
                        .collect::<Vec<_>>()
                        .into(),
                ))
            }
            Expr::Scan(list) => {
                let Value::List(values) = self.eval(*list, inputs, arguments)? else {
                    unreachable!("ill-typed spec operand");
                };
                self.fuel(1)?;
                self.fuel(values.len() as u64)?;
                let bits: Vec<_> = values
                    .iter()
                    .map(|value| {
                        let Value::F32(bits) = *value else {
                            unreachable!("ill-typed spec element")
                        };
                        bits
                    })
                    .collect();
                Ok(Value::U64(reference_min(&bits)))
            }
            Expr::At(list, index) => {
                let Value::List(values) = self.eval(*list, inputs, arguments)? else {
                    unreachable!("ill-typed spec operand");
                };
                let Value::U64(index) = self.eval(*index, inputs, arguments)? else {
                    unreachable!("ill-typed spec index");
                };
                self.fuel(1)?;
                usize::try_from(index)
                    .ok()
                    .and_then(|index| values.get(index))
                    .cloned()
                    .ok_or(INDEX)
            }
        }
    }
}

// Order encodings as integers, coalescing both zeros and excluding NaNs.
// Taking the least (key, index) pair keeps the first tie.
fn ordered(bits: u32) -> Option<u32> {
    let magnitude = bits & 0x7fff_ffff;
    if magnitude > 0x7f80_0000 {
        None
    } else if magnitude == 0 {
        Some(0x7fff_ffff)
    } else if bits >> 31 == 0 {
        Some(bits | 0x8000_0000)
    } else {
        Some(!bits)
    }
}

pub(super) fn reference_min(bits: &[u32]) -> u64 {
    if bits.first().is_some_and(|&first| ordered(first).is_none()) {
        return 0;
    }
    bits.iter()
        .enumerate()
        .filter_map(|(index, &bits)| Some((ordered(bits)?, index)))
        .min()
        .map_or(0, |(_, index)| index as u64)
}
