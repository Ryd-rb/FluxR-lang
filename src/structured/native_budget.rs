use super::compile::Op;
use super::{Limits, Module, Type};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default)]
struct Work {
    fuel: u64,
    cells: usize,
    frames: usize,
    list_items: usize,
    pushes: usize,
    scans: u64,
}

impl Module {
    /// Upper bound for native execution of a forward-only export.
    ///
    /// The caller must validate types, bound the recursive cells in all inputs
    /// and arguments, and separately verify native code identity. An unsupported
    /// instruction, cycle or overflowing bound requires the metered VM.
    pub fn native_ceiling(&self, export: &str, input_cells: usize) -> Option<Limits> {
        self.native_work_ceiling(export, input_cells, None)
    }

    /// Upper bound when a matching native emitter enforces a shared quota
    /// on loop-body entries. Every taken backedge spends an entry first.
    pub fn native_loop_ceiling(
        &self,
        export: &str,
        input_cells: usize,
        max_backedges: usize,
    ) -> Option<Limits> {
        self.native_work_ceiling(export, input_cells, Some(max_backedges))
    }

    fn native_work_ceiling(
        &self,
        export: &str,
        input_cells: usize,
        max_backedges: Option<usize>,
    ) -> Option<Limits> {
        let export = *self.exports.get(export)?;
        let work = self.forward_work(
            export,
            max_backedges,
            &mut vec![false; self.code.len()],
            &mut vec![None; self.code.len()],
        )?;
        // Each live list starts in the inputs or is constructed by the code.
        // Copies can duplicate a list, but only allocations and pushes grow it.
        let max_list = input_cells.checked_add(work.list_items)?;
        let mut shapes = BTreeMap::new();
        let result_cells = match &self.code[export].result {
            Type::List(element) => max_list
                .checked_mul(self.fixed_value_cells(element, &mut shapes)?)?
                .checked_add(1)?,
            other => self.fixed_value_cells(other, &mut shapes)?,
        };
        Some(Limits {
            instructions: work
                .fuel
                .checked_add(work.scans.checked_mul(u64::try_from(max_list).ok()?)?)?,
            cells: input_cells
                .checked_add(result_cells)?
                .checked_add(work.cells)?
                .checked_add(work.pushes.checked_mul(max_list.checked_add(1)?)?)?,
            frames: work.frames,
        })
    }

    fn forward_work(
        &self,
        function: usize,
        max_backedges: Option<usize>,
        active: &mut [bool],
        cache: &mut [Option<Work>],
    ) -> Option<Work> {
        if let Some(work) = cache[function] {
            return Some(work);
        }
        if active[function] {
            return None;
        }
        active[function] = true;
        let code = &self.code[function];
        let mut work = Work {
            cells: code.slots.checked_add(1)?,
            frames: 1,
            ..Work::default()
        };
        let mut has_backedge = false;
        // Count both branches. Each instruction in this activation runs at
        // most one extra time per backedge, including each call site.
        for (pc, instruction) in code.ops.iter().enumerate() {
            work.fuel = work.fuel.checked_add(instruction.fuel)?;
            let cells = match &instruction.op {
                Op::Record(fields) => fields.checked_add(1)?,
                Op::Variant(..) => 1,
                Op::List(count) => {
                    work.list_items = work.list_items.checked_add(*count)?;
                    count.checked_add(1)?
                }
                Op::Push(_) => {
                    work.pushes = work.pushes.checked_add(1)?;
                    work.list_items = work.list_items.checked_add(1)?;
                    0
                }
                Op::ArgMinF32 => {
                    work.scans = work.scans.checked_add(1)?;
                    0
                }
                Op::Call(callee) => {
                    let nested = self.forward_work(*callee, max_backedges, active, cache)?;
                    work.fuel = work.fuel.checked_add(nested.fuel)?;
                    work.frames = work.frames.max(nested.frames.checked_add(1)?);
                    work.list_items = work.list_items.checked_add(nested.list_items)?;
                    work.pushes = work.pushes.checked_add(nested.pushes)?;
                    work.scans = work.scans.checked_add(nested.scans)?;
                    nested.cells
                }
                Op::Jump(to) | Op::JumpTrue(to) | Op::JumpFalse(to) if *to > pc => 0,
                Op::Jump(to) if *to < pc && max_backedges.is_some() => {
                    has_backedge = true;
                    0
                }
                Op::Operand(_)
                | Op::Store(_)
                | Op::Binary(_)
                | Op::BinaryOperands(..)
                | Op::Unary(_)
                | Op::Field(_)
                | Op::Index
                | Op::Return => 0,
                _ => return None,
            };
            work.cells = work.cells.checked_add(cells)?;
        }
        if has_backedge {
            let passes = max_backedges?.checked_add(1)?;
            let fuel_passes = u64::try_from(passes).ok()?;
            work.fuel = work.fuel.checked_mul(fuel_passes)?;
            work.cells = work.cells.checked_mul(passes)?;
            work.list_items = work.list_items.checked_mul(passes)?;
            work.pushes = work.pushes.checked_mul(passes)?;
            work.scans = work.scans.checked_mul(fuel_passes)?;
        }
        active[function] = false;
        cache[function] = Some(work);
        Some(work)
    }

    fn fixed_value_cells<'a>(
        &'a self,
        ty: &'a Type,
        shapes: &mut BTreeMap<&'a str, usize>,
    ) -> Option<usize> {
        let name = match ty {
            Type::List(_) => return None,
            Type::Record(name) | Type::Enum(name) => name,
            _ => return Some(1),
        };
        if let Some(&size) = shapes.get(name.as_str()) {
            return Some(size);
        }
        let size = if let Some(record) = self.records.get(name) {
            record.fields.iter().try_fold(1usize, |sum, field| {
                sum.checked_add(self.fixed_value_cells(&field.ty, shapes)?)
            })?
        } else {
            self.enums[name]
                .variants
                .iter()
                .try_fold(1, |max, variant| {
                    let Some(payload) = &variant.payload else {
                        return Some(max);
                    };
                    Some(max.max(self.fixed_value_cells(payload, shapes)?.checked_add(1)?))
                })?
        };
        shapes.insert(name, size);
        Some(size)
    }
}
