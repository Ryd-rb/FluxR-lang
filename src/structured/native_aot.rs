//! Ahead-of-time Rust emission from the same metered IR as the JIT.
use super::native_ir::{Boundary, Exit, MAX_LANES, Plan, Step};
use super::*;
use sha2::{Digest, Sha256};
use std::fmt::Write;

type Kernel = fn(&[i64; MAX_LANES], &mut [i64; MAX_LANES], u64, u64) -> u32;

#[cfg(test)]
mod control_flow {
    use super::*;
    use crate as fluxr;

    include!("../../examples/fixtures/control_flow_aot.rs");

    #[test]
    fn exact_control_flow_can_complete_without_portable_fallback() {
        let module = Module::compile_compact(include_str!(
            "../../examples/fixtures/control_flow.fluxr"
        ))
        .unwrap();
        let aot = prepare(&module).unwrap();
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        let jit = super::super::typed_jit::Native::compile(&module, module.exports["walk"]).unwrap();
        for (n, step, fuel, cells, total) in [(0, 0, 6, 13, 7), (4, 2, 60, 18, 6)] {
            let state = Value::Record([Value::I64(7), Value::I64(-11)].into());
            let args = [state, Value::I64(n), Value::I64(step)];
            let call = Invocation {
                export: "walk",
                inputs: &[],
                arguments: &args,
                // The native boundary reserves the export's maximum depth
                // even when the early return skips the helper.
                limits: Limits {
                    instructions: fuel,
                    cells,
                    frames: 2,
                },
            };
            let expected = Value::Record([Value::I64(total), Value::I64(-11)].into());
            assert_eq!(aot.boundary.run(&call, aot.kernel), Some(expected.clone()));
            #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
            assert_eq!(jit.run(&call), Some(expected));
        }
    }
}

#[cfg(test)]
mod loop_exits {
    use super::*;
    use crate as fluxr;

    include!("../../examples/fixtures/loop_exits_aot.rs");

    #[test]
    fn exact_loop_exits_complete_without_portable_fallback() {
        let module =
            Module::compile_compact(include_str!("../../examples/fixtures/loop_exits.fluxr"))
                .unwrap();
        let aot = prepare(&module).unwrap();
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        let jit =
            super::super::typed_jit::Native::compile(&module, module.exports["walk"]).unwrap();
        for (n, stop, skip, step, instructions, cells, total) in [
            (0, 0, 0, 0, 14, 17, 7),
            (1, 0, 1, 0, 23, 17, 7),
            (1, -1, 1, 0, 31, 17, 7),
            (4, 0, 2, 6, 88, 19, 12),
            (1024, -1, 1024, 0, 17_422, 17, 7),
        ] {
            let state = Value::Record([Value::I64(7), Value::I64(-11)].into());
            let [n, stop, skip, step] = [n, stop, skip, step].map(Value::I64);
            let args = [state, n, stop, skip, step];
            let call = Invocation {
                export: "walk",
                inputs: &[],
                arguments: &args,
                limits: Limits {
                    instructions,
                    cells,
                    frames: 1,
                },
            };
            let expected = Value::Record([Value::I64(total), Value::I64(-11)].into());
            assert_eq!(aot.boundary.run(&call, aot.kernel), Some(expected.clone()));
            #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
            assert_eq!(jit.run(&call), Some(expected));
        }
    }
}

#[cfg(test)]
mod nested_loop_calls {
    use super::*;
    use crate as fluxr;

    include!("../../examples/fixtures/nested_loop_calls_aot.rs");

    #[test]
    fn nested_loops_reserve_maximum_native_depth() {
        let module = Module::compile_compact(include_str!(
            "../../examples/fixtures/nested_loop_calls.fluxr"
        ))
        .unwrap();
        let aot = prepare(&module).unwrap();
        #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
        let jit =
            super::super::typed_jit::Native::compile(&module, module.exports["walk"]).unwrap();
        for (scalars, instructions, cells, total) in [
            ([1, 4, 2, 6, 0], 23, 20, 7),
            ([3, 0, 2, 6, 0], 69, 22, 8),
            ([2, 4, 3, 6, 0], 99, 22, 8),
            ([2, 4, 2, 6, 2], 110, 25, 11),
            ([2, 4, 2, -1, 1], 108, 23, 7),
            ([1024, 4, 2, 0, 1], 86_912, 3088, 523_781),
        ] {
            let host = [Value::I64(7), Value::I64(-11)].into();
            let [n, width, trigger, step, divisor] = scalars.map(Value::I64);
            let args = [Value::Record(host), n, width, trigger, step, divisor];
            let call = Invocation {
                export: "walk",
                inputs: &[],
                arguments: &args,
                limits: Limits {
                    instructions,
                    cells,
                    frames: 2,
                },
            };
            let expected = Value::Record([Value::I64(total), Value::I64(-11)].into());
            assert_eq!(aot.boundary.run(&call, aot.kernel), Some(expected.clone()));
            #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
            assert_eq!(jit.run(&call), Some(expected));
            let shallow = Invocation {
                limits: Limits {
                    frames: 1,
                    ..call.limits
                },
                ..call
            };
            assert_eq!(aot.boundary.run(&shallow, aot.kernel), None);
            #[cfg(all(feature = "typed-jit", target_os = "windows", target_arch = "x86_64"))]
            assert_eq!(jit.run(&shallow), None);
        }
    }
}

pub struct PreparedAot<'module> {
    module: &'module Module,
    export: &'module str,
    boundary: Boundary,
    kernel: Kernel,
}

impl PreparedAot<'_> {
    pub fn run_with(
        &self,
        call: Invocation<'_>,
        workspace: &mut Workspace,
    ) -> Result<Value, Error> {
        if call.export == self.export
            && let Some(result) = self.boundary.run(&call, self.kernel)
        {
            return Ok(result);
        }
        self.module.run_portable_with(call, workspace)
    }
}

impl Module {
    /// Emit Rust for a supported export. The generated module includes a
    /// preparation function that binds the kernel to this artifact and export.
    pub fn emit_native_rust(&self, export: &str) -> Result<String, Error> {
        let plan = self.aot_plan(export)?;
        Ok(emit(&plan, export, self.aot_identity(export)))
    }

    /// Bind trusted generated Rust to a checked module for repeated calls.
    /// The digest covers source identity, export and ABI version. The host must link
    /// the kernel emitted alongside that digest.
    pub fn prepare_aot<'module>(
        &'module self,
        export: &'module str,
        identity: [u8; 32],
        kernel: Kernel,
    ) -> Result<PreparedAot<'module>, Error> {
        if self.aot_identity(export) != identity {
            return Err(error("native AOT module or export identity differs"));
        }
        Ok(PreparedAot {
            module: self,
            export,
            boundary: self.aot_plan(export)?.boundary,
            kernel,
        })
    }

    fn aot_identity(&self, export: &str) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(b"fluxr/typed-aot/1\0");
        sha.update((export.len() as u64).to_le_bytes());
        sha.update(export.as_bytes());
        sha.update(self.encode());
        sha.finalize().into()
    }

    fn aot_plan(&self, export: &str) -> Result<Plan, Error> {
        self.exports
            .get(export)
            .and_then(|&index| Plan::lower(self, index))
            .ok_or_else(|| error(format!("unsupported native AOT export {export}")))
    }
}

fn emit(plan: &Plan, export: &str, identity: [u8; 32]) -> String {
    let mut out = String::new();
    let dynamic = plan.boundary.fixed.is_none();
    writeln!(out, "// Generated by FluxR emit-native-structured.").unwrap();
    writeln!(out, "pub fn prepare(module: &fluxr::structured::Module) -> Result<fluxr::structured::PreparedAot<'_>, fluxr::Error> {{").unwrap();
    writeln!(
        out,
        "    module.prepare_aot({export:?}, {identity:?}, kernel)"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "fn kernel(input: &[i64; {MAX_LANES}], output: &mut [i64; {MAX_LANES}], mut fuel: u64, mut cells: u64) -> u32 {{").unwrap();
    let mut lane = 0;
    for (index, shape) in plan
        .boundary
        .inputs
        .iter()
        .chain(&plan.boundary.args)
        .enumerate()
    {
        let values = (lane..lane + shape.lanes())
            .map(|n| format!("input[{n}]"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "    let in{index} = [{values}];").unwrap();
        lane += shape.lanes();
    }
    let args = (plan.boundary.inputs.len()..plan.boundary.inputs.len() + plan.boundary.args.len())
        .map(|i| format!(", in{i}"))
        .collect::<String>();
    let inputs = (0..plan.boundary.inputs.len())
        .map(|i| format!(", in{i}"))
        .collect::<String>();
    writeln!(
        out,
        "    match f{}(&mut fuel, &mut cells{inputs}{args}) {{",
        plan.entry
    )
    .unwrap();
    writeln!(
        out,
        "        Some(value) => {{ output[..{}].copy_from_slice(&value); 0 }}",
        plan.boundary.result.lanes()
    )
    .unwrap();
    writeln!(out, "        None => 1,").unwrap();
    writeln!(out, "    }}\n}}").unwrap();

    for (index, function) in plan.functions.iter().enumerate() {
        let Some(function) = function else { continue };
        let mut params = String::new();
        for (index, shape) in plan.boundary.inputs.iter().enumerate() {
            write!(params, ", in{index}: [i64; {}]", shape.lanes()).unwrap();
        }
        for (index, shape) in function.slots.iter().take(function.params).enumerate() {
            write!(params, ", mut l{index}: [i64; {}]", shape.lanes()).unwrap();
        }
        writeln!(out, "#[inline(always)]").unwrap();
        writeln!(out, "#[allow(unused_variables, unused_mut, unused_assignments, clippy::too_many_arguments, clippy::needless_return)]").unwrap();
        writeln!(
            out,
            "fn f{index}(fuel: &mut u64, cells: &mut u64{params}) -> Option<[i64; {}]> {{",
            function.result.lanes()
        )
        .unwrap();
        for (index, shape) in function.slots.iter().enumerate().skip(function.params) {
            writeln!(out, "    let mut l{index} = [0i64; {}];", shape.lanes()).unwrap();
            writeln!(out, "    let mut init{index} = false;").unwrap();
        }
        let branch = function.blocks.len() > 1;
        if branch {
            writeln!(
                out,
                "    let mut block = 0;\n    loop {{\n        match block {{"
            )
            .unwrap();
        }
        for (index, block) in function.blocks.iter().enumerate() {
            let indent = if branch { "                " } else { "    " };
            if branch {
                writeln!(out, "            {index} => {{").unwrap();
            }
            if dynamic && block.fuel != 0 {
                writeln!(out, "{indent}*fuel = fuel.checked_sub({})?;", block.fuel).unwrap();
            }
            if dynamic && block.cells != 0 {
                writeln!(out, "{indent}*cells = cells.checked_sub({})?;", block.cells).unwrap();
            }
            for (temp, step) in block.steps.iter().enumerate() {
                let expression = match step {
                    Step::Const(value) => format!("[{value}i64]"),
                    Step::Input(index) => format!("in{index}"),
                    Step::Load(slot) => {
                        if *slot >= function.params {
                            writeln!(out, "{indent}if !init{slot} {{ return None; }}").unwrap();
                        }
                        format!("l{slot}")
                    }
                    Step::Store(slot, value) => {
                        writeln!(out, "{indent}l{slot} = t{value};").unwrap();
                        if *slot >= function.params {
                            writeln!(out, "{indent}init{slot} = true;").unwrap();
                        }
                        continue;
                    }
                    Step::Update(slot, field, value) => {
                        if *slot >= function.params {
                            writeln!(out, "{indent}if !init{slot} {{ return None; }}").unwrap();
                        }
                        writeln!(
                            out,
                            "{indent}l{slot}[0] = 0;\n{indent}l{slot}[{}] = t{value}[0];",
                            field + 1
                        )
                        .unwrap();
                        continue;
                    }
                    Step::Field(record, field) => format!("[t{record}[{}]]", field + 1),
                    Step::Binary(kind, left, right) => binary(*kind, *left, *right),
                    Step::Call(index, args) => {
                        let args = args
                            .iter()
                            .map(|temp| format!(", t{temp}"))
                            .collect::<String>();
                        format!("f{index}(fuel, cells{inputs}{args})?")
                    }
                };
                writeln!(out, "{indent}let t{temp} = {expression};").unwrap();
            }
            let exit = match block.exit {
                Exit::Return(temp) => format!("return Some(t{temp});"),
                Exit::Jump(target) => format!("block = {target};"),
                Exit::Branch(temp, yes, no) => {
                    format!("block = if t{temp}[0] != 0 {{ {yes} }} else {{ {no} }};")
                }
            };
            writeln!(out, "{indent}{exit}").unwrap();
            if branch {
                writeln!(out, "            }}").unwrap();
            }
        }
        if branch {
            writeln!(out, "            _ => return None,\n        }}\n    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
    out
}

fn binary(kind: Binary, left: usize, right: usize) -> String {
    use Binary::*;
    let method = match kind {
        Add => "add",
        Sub => "sub",
        Mul => "mul",
        Div => "div",
        Rem => "rem",
        Eq | Lt | Le => {
            let symbol = match kind {
                Eq => "==",
                Lt => "<",
                _ => "<=",
            };
            return format!("[i64::from(t{left}[0] {symbol} t{right}[0])]");
        }
        _ => unreachable!("lowered signed operation"),
    };
    format!("[t{left}[0].checked_{method}(t{right}[0])?]")
}
