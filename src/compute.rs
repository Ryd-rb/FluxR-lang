//! VKF1: an opt-in, bounded lowering of checked FluxR expressions to SPIR-V.
//!
//! This profile is deliberately separate from released FSM execution. It admits
//! one pure f32 return expression using parameters, finite literals and +, -, *.
//! GPU numerical behavior is explicitly approximate; the portable VM remains
//! the reference. No calls, loops, effects, atomics or barriers are inferred.
use crate::Error;
use crate::structured::{Binary, Expr, Invocation, Limits, Module, Stmt, Type, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const PROFILE: &str = "fluxr-vkf1-finite-f32-nocontract";
pub const SPECIALIZED_PROFILE: &str = "fluxr-vks1-finite-f32-nocontract";
pub const MAX_PARAMETERS: usize = 8;
pub const MAX_OPERATIONS: usize = 128;

/// An exact finite f32 value for an original FluxR parameter. Its zero-based
/// parameter index is also its SPIR-V SpecId; ordering of caller entries is not
/// semantic. Signed zero is retained. At least one parameter remains a buffer.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Specialization {
    pub parameter_index: u32,
    pub value_bits: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Reflection {
    pub profile: &'static str,
    pub source_sha256: String,
    pub spirv_sha256: String,
    pub export: String,
    pub entry_point: &'static str,
    pub input_bindings: usize,
    pub output_binding: usize,
    pub descriptor_set: u32,
    pub array_stride_bytes: u32,
    pub push_constant_bytes: u32,
    pub workgroup: [u32; 3],
    pub arithmetic_operations: usize,
    pub bounds: &'static str,
    pub numerical_policy: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub specializations: Vec<Specialization>,
    /// Source and template hashes alone do not identify specialized execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_sha256: Option<String>,
}

/// Immutable checked compiler output. Arbitrary SPIR-V cannot enter the safe
/// executor through this type. `words` is available for independent validation.
#[derive(Clone)]
pub struct Kernel {
    words: Vec<u32>,
    reflection: Reflection,
    module: Module,
    source_bytes: Vec<u8>,
}

fn error(message: &str) -> Error {
    Error(message.into())
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl Kernel {
    pub fn compile(module: &Module, export: &str, workgroup_size: u32) -> Result<Self, Error> {
        Self::compile_profile(module, export, workgroup_size, &[])
    }

    /// Additive VKS1 profile: specialize 1..=7 finite f32 parameters at pipeline
    /// creation. Dynamic buffers retain original parameter order with the
    /// specialized indices removed. VKF1 compile and shader bytes are unchanged.
    pub fn compile_specialized(
        module: &Module,
        export: &str,
        workgroup_size: u32,
        specializations: &[Specialization],
    ) -> Result<Self, Error> {
        if specializations.is_empty() {
            return Err(error("VKS1 requires at least one specialization"));
        }
        Self::compile_profile(module, export, workgroup_size, specializations)
    }

    fn compile_profile(
        module: &Module,
        export: &str,
        workgroup_size: u32,
        specializations: &[Specialization],
    ) -> Result<Self, Error> {
        // Admission precedes cloning caller-controlled metadata.
        if specializations.len() >= MAX_PARAMETERS {
            return Err(error("VKS1 specialization count exceeded"));
        }
        if !(1..=256).contains(&workgroup_size) {
            return Err(error("VKF1 workgroup must be in 1..=256"));
        }
        let source = module.checked_source()?;
        if !source.inputs.is_empty() {
            return Err(error("VKF1 does not admit host inputs or effects"));
        }
        let function = source
            .functions
            .iter()
            .find(|function| function.export && function.name == export)
            .ok_or_else(|| error("VKF1 export not found"))?;
        if function.receiver.is_some()
            || function.result != Type::F32
            || !function.locals.is_empty()
            || !(1..=MAX_PARAMETERS).contains(&function.params.len())
            || function.params.iter().any(|p| p.ty != Type::F32)
        {
            return Err(error(
                "VKF1 expects 1..=8 f32 parameters, f32 result, no locals",
            ));
        }
        let [Stmt::Return { value }] = function.body.as_slice() else {
            return Err(error("VKF1 requires one pure return expression"));
        };
        let mut specializations = specializations.to_vec();
        specializations.sort_by_key(|s| s.parameter_index);
        if specializations.len() >= function.params.len()
            || specializations.iter().any(|s| {
                s.parameter_index as usize >= function.params.len()
                    || !f32::from_bits(s.value_bits).is_finite()
            })
            || specializations
                .windows(2)
                .any(|p| p[0].parameter_index == p[1].parameter_index)
        {
            return Err(error(
                "VKS1 requires unique in-range finite constants and a dynamic input",
            ));
        }
        let mut emitter = Emitter::new();
        let void = emitter.id();
        let bool_ty = emitter.id();
        let uint = emitter.id();
        let float = emitter.id();
        let vec3 = emitter.id();
        let array = emitter.id();
        let buffer = emitter.id();
        let ptr_buffer = emitter.id();
        let ptr_float = emitter.id();
        let ptr_vec3 = emitter.id();
        let push_struct = emitter.id();
        let ptr_push = emitter.id();
        let ptr_push_uint = emitter.id();
        let function_ty = emitter.id();
        let zero = emitter.id();
        let gid = emitter.id();
        let push = emitter.id();
        let inputs: Vec<_> = (0..function.params.len() - specializations.len())
            .map(|_| emitter.id())
            .collect();
        let output = emitter.id();
        let main = emitter.id();
        let start = emitter.id();
        let active = emitter.id();
        let merge = emitter.id();

        op(&mut emitter.header, 17, &[1]); // Shader
        op(&mut emitter.header, 14, &[0, 1]); // Logical, GLSL450
        let mut entry = vec![5, main]; // GLCompute
        entry.extend(string_words("main"));
        entry.push(gid);
        op(&mut emitter.header, 15, &entry);
        op(&mut emitter.header, 16, &[main, 17, workgroup_size, 1, 1]);
        op(&mut emitter.annotations, 71, &[gid, 11, 28]); // BuiltIn GlobalInvocationId
        op(&mut emitter.annotations, 71, &[array, 6, 4]); // ArrayStride
        op(&mut emitter.annotations, 71, &[buffer, 3]); // BufferBlock
        op(&mut emitter.annotations, 72, &[buffer, 0, 35, 0]);
        op(&mut emitter.annotations, 71, &[push_struct, 2]); // Block
        op(&mut emitter.annotations, 72, &[push_struct, 0, 35, 0]);
        for (binding, &variable) in inputs.iter().chain([output].iter()).enumerate() {
            op(&mut emitter.annotations, 71, &[variable, 34, 0]);
            op(
                &mut emitter.annotations,
                71,
                &[variable, 33, binding as u32],
            );
            op(
                &mut emitter.annotations,
                71,
                &[variable, if variable == output { 25 } else { 24 }],
            );
        }
        op(&mut emitter.types, 19, &[void]);
        op(&mut emitter.types, 20, &[bool_ty]);
        op(&mut emitter.types, 21, &[uint, 32, 0]);
        op(&mut emitter.types, 22, &[float, 32]);
        op(&mut emitter.types, 23, &[vec3, uint, 3]);
        op(&mut emitter.types, 29, &[array, float]);
        op(&mut emitter.types, 30, &[buffer, array]);
        op(&mut emitter.types, 32, &[ptr_buffer, 2, buffer]);
        op(&mut emitter.types, 32, &[ptr_float, 2, float]);
        op(&mut emitter.types, 32, &[ptr_vec3, 1, vec3]);
        op(&mut emitter.types, 30, &[push_struct, uint]);
        op(&mut emitter.types, 32, &[ptr_push, 9, push_struct]);
        op(&mut emitter.types, 32, &[ptr_push_uint, 9, uint]);
        op(&mut emitter.types, 33, &[function_ty, void]);
        op(&mut emitter.types, 43, &[uint, zero, 0]);
        op(&mut emitter.types, 59, &[ptr_vec3, gid, 1]);
        op(&mut emitter.types, 59, &[ptr_push, push, 9]);
        for &variable in inputs.iter().chain([output].iter()) {
            op(&mut emitter.types, 59, &[ptr_buffer, variable, 2]);
        }
        op(&mut emitter.body, 54, &[void, main, 0, function_ty]);
        op(&mut emitter.body, 248, &[start]);
        let invocation = emitter.id();
        let index = emitter.id();
        let count_ptr = emitter.id();
        let count = emitter.id();
        let inside = emitter.id();
        op(&mut emitter.body, 61, &[vec3, invocation, gid]);
        op(&mut emitter.body, 81, &[uint, index, invocation, 0]);
        op(
            &mut emitter.body,
            65,
            &[ptr_push_uint, count_ptr, push, zero],
        );
        op(&mut emitter.body, 61, &[uint, count, count_ptr]);
        op(&mut emitter.body, 176, &[bool_ty, inside, index, count]);
        op(&mut emitter.body, 247, &[merge, 0]);
        op(&mut emitter.body, 250, &[inside, active, merge]);
        op(&mut emitter.body, 248, &[active]);
        let mut parameters = BTreeMap::new();
        let mut dynamic = inputs.iter();
        for (parameter_index, param) in function.params.iter().enumerate() {
            if specializations
                .iter()
                .any(|s| s.parameter_index as usize == parameter_index)
            {
                let constant = emitter.id();
                op(
                    &mut emitter.annotations,
                    71,
                    &[constant, 1, parameter_index as u32],
                ); // SpecId
                // A common template uses +0 defaults. The executor MUST supply
                // every exact value in reflection when creating the pipeline.
                op(&mut emitter.types, 50, &[float, constant, 0]); // OpSpecConstant
                parameters.insert(param.name.as_str(), constant);
                continue;
            }
            let variable = *dynamic.next().expect("checked dynamic parameter count");
            let pointer = emitter.id();
            let loaded = emitter.id();
            op(
                &mut emitter.body,
                65,
                &[ptr_float, pointer, variable, zero, index],
            );
            op(&mut emitter.body, 61, &[float, loaded, pointer]);
            parameters.insert(param.name.as_str(), loaded);
        }
        let result = emitter.expression(value, float, &parameters, 0)?;
        let output_ptr = emitter.id();
        op(
            &mut emitter.body,
            65,
            &[ptr_float, output_ptr, output, zero, index],
        );
        op(&mut emitter.body, 62, &[output_ptr, result]);
        op(&mut emitter.body, 249, &[merge]);
        op(&mut emitter.body, 248, &[merge]);
        op(&mut emitter.body, 253, &[]);
        op(&mut emitter.body, 56, &[]);
        let operations = emitter.operations;
        let words = emitter.finish();
        let source_bytes = module.encode();
        let mut reflection = Reflection {
            profile: if specializations.is_empty() {
                PROFILE
            } else {
                SPECIALIZED_PROFILE
            },
            source_sha256: sha256(&source_bytes),
            spirv_sha256: sha256(
                &words
                    .iter()
                    .flat_map(|x| x.to_le_bytes())
                    .collect::<Vec<_>>(),
            ),
            export: export.into(),
            entry_point: "main",
            input_bindings: inputs.len(),
            output_binding: inputs.len(),
            descriptor_set: 0,
            array_stride_bytes: 4,
            push_constant_bytes: 4,
            workgroup: [workgroup_size, 1, 1],
            arithmetic_operations: operations,
            bounds: "global_invocation_id.x < push.count before any storage access",
            numerical_policy: "explicit approximate GPU f32; finite inputs; NoContraction; CPU VM reference retained; no cross-device bitwise guarantee",
            specializations,
            variant_sha256: None,
        };
        if !reflection.specializations.is_empty() {
            let mut identity = Sha256::new();
            identity.update(b"fluxr-vks1-variant-v1\0");
            identity.update(reflection.source_sha256.as_bytes());
            identity.update(reflection.spirv_sha256.as_bytes());
            identity.update((export.len() as u64).to_le_bytes());
            identity.update(export.as_bytes());
            for s in &reflection.specializations {
                identity.update(s.parameter_index.to_le_bytes());
                identity.update(s.value_bits.to_le_bytes());
            }
            reflection.variant_sha256 = Some(format!("{:x}", identity.finalize()));
        }
        Ok(Self {
            words,
            reflection,
            module: module.clone(),
            source_bytes,
        })
    }

    /// Canonical FSM bytes used by the immutable source identity.
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub fn words(&self) -> &[u32] {
        &self.words
    }
    pub fn reflection(&self) -> &Reflection {
        &self.reflection
    }
    pub fn bytes(&self) -> Vec<u8> {
        self.words.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    pub fn check_inputs(&self, inputs: &[&[f32]], max_elements: usize) -> Result<usize, Error> {
        if inputs.len() != self.reflection.input_bindings {
            return Err(error("VKF1 input binding count mismatch"));
        }
        let count = inputs[0].len();
        if count == 0 || count > max_elements || count > u32::MAX as usize {
            return Err(error("VKF1 element budget exceeded or empty dispatch"));
        }
        if inputs
            .iter()
            .any(|input| input.len() != count || input.iter().any(|x| !x.is_finite()))
        {
            return Err(error("VKF1 expects equal-length finite f32 inputs"));
        }
        Ok(count)
    }

    /// Execute the original checked FluxR module, preserving its operation order.
    pub fn cpu_reference(&self, inputs: &[&[f32]], max_elements: usize) -> Result<Vec<f32>, Error> {
        let count = self.check_inputs(inputs, max_elements)?;
        let mut output = Vec::with_capacity(count);
        for i in 0..count {
            let mut dynamic = inputs.iter();
            let args: Vec<_> = (0..inputs.len() + self.reflection.specializations.len())
                .map(|index| {
                    let bits = self
                        .reflection
                        .specializations
                        .iter()
                        .find(|s| s.parameter_index as usize == index)
                        .map(|s| s.value_bits)
                        .unwrap_or_else(|| {
                            dynamic.next().expect("checked binding count")[i].to_bits()
                        });
                    Value::F32(bits)
                })
                .collect();
            match self.module.run(Invocation {
                export: &self.reflection.export,
                inputs: &[],
                arguments: &args,
                limits: Limits {
                    instructions: 2048,
                    cells: 256,
                    frames: 1,
                },
            })? {
                Value::F32(bits) => output.push(f32::from_bits(bits)),
                _ => return Err(error("VKF1 internal result type mismatch")),
            }
        }
        Ok(output)
    }
}

struct Emitter {
    next: u32,
    header: Vec<u32>,
    annotations: Vec<u32>,
    types: Vec<u32>,
    body: Vec<u32>,
    operations: usize,
}
impl Emitter {
    fn new() -> Self {
        Self {
            next: 1,
            header: vec![],
            annotations: vec![],
            types: vec![],
            body: vec![],
            operations: 0,
        }
    }
    fn id(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
    fn expression(
        &mut self,
        value: &Expr,
        float: u32,
        parameters: &BTreeMap<&str, u32>,
        depth: usize,
    ) -> Result<u32, Error> {
        if depth > 32 {
            return Err(error("VKF1 expression depth exceeded"));
        }
        match value {
            Expr::F32 { bits } if f32::from_bits(*bits).is_finite() => {
                let result = self.id();
                op(&mut self.types, 43, &[float, result, *bits]);
                Ok(result)
            }
            Expr::Local { name } => parameters
                .get(name.as_str())
                .copied()
                .ok_or_else(|| error("VKF1 unknown parameter")),
            Expr::Binary { kind, left, right } => {
                let opcode = match kind {
                    Binary::Add => 129,
                    Binary::Sub => 131,
                    Binary::Mul => 133,
                    _ => return Err(error("VKF1 unsupported binary operation")),
                };
                self.operations += 1;
                if self.operations > MAX_OPERATIONS {
                    return Err(error("VKF1 arithmetic operation budget exceeded"));
                }
                let a = self.expression(left, float, parameters, depth + 1)?;
                let b = self.expression(right, float, parameters, depth + 1)?;
                let result = self.id();
                op(&mut self.annotations, 71, &[result, 42]); // NoContraction
                op(&mut self.body, opcode, &[float, result, a, b]);
                Ok(result)
            }
            _ => Err(error("VKF1 unsupported expression or nonfinite literal")),
        }
    }
    fn finish(self) -> Vec<u32> {
        let mut words = vec![0x07230203, 0x00010000, 0, self.next, 0];
        words.extend(self.header);
        words.extend(self.annotations);
        words.extend(self.types);
        words.extend(self.body);
        words
    }
}
fn op(out: &mut Vec<u32>, opcode: u32, args: &[u32]) {
    out.push(((args.len() as u32 + 1) << 16) | opcode);
    out.extend_from_slice(args);
}
fn string_words(value: &str) -> Vec<u32> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|x| u32::from_le_bytes(*x))
        .collect()
}
