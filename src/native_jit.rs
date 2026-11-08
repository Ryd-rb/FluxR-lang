//! Experimental Windows x86-64 lowering for validated rules.
use super::{Binary, Error, MAX_STACK, Op, RuleProgram};
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

const CODE_LIMIT: usize = 16 * 1024;
const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READ: u32 = 0x20;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn VirtualFree(addr: *mut c_void, size: usize, ty: u32) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
}

struct Page(NonNull<c_void>);

// SAFETY: Published pages are immutable RX mappings. The sole owner releases
// the mapping on drop; shared evaluation borrows that owner and uses local scratch.
unsafe impl Send for Page {}
// SAFETY: The generated leaf function only reads its inputs and writes local scratch.
unsafe impl Sync for Page {}

impl Drop for Page {
    fn drop(&mut self) {
        // SAFETY: The base is from VirtualAlloc and released exactly once.
        unsafe { VirtualFree(self.0.as_ptr(), 0, MEM_RELEASE) };
    }
}

impl Page {
    fn publish(code: &[u8]) -> Result<Self, Error> {
        debug_assert!(!code.is_empty() && code.len() <= CODE_LIMIT);
        // SAFETY: Allocate a private, initially writable mapping for these bytes.
        let ptr = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                code.len(),
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            )
        };
        let page = Self(NonNull::new(ptr).ok_or_else(|| platform_error("VirtualAlloc"))?);
        let mut old = 0;
        // SAFETY: The distinct destination is writable for code.len() bytes and
        // old is a valid local out pointer. The mapping is never writable and executable.
        unsafe {
            std::ptr::copy_nonoverlapping(code.as_ptr(), ptr.cast::<u8>(), code.len());
            if VirtualProtect(ptr, code.len(), PAGE_EXECUTE_READ, &mut old) == 0 {
                return Err(platform_error("VirtualProtect"));
            }
            if FlushInstructionCache(GetCurrentProcess(), ptr, code.len()) == 0 {
                return Err(platform_error("FlushInstructionCache"));
            }
        }
        Ok(page)
    }
}

fn platform_error(operation: &str) -> Error {
    Error(format!(
        "native rule {operation}: {}",
        std::io::Error::last_os_error()
    ))
}

pub(super) struct NativeRule {
    page: Page,
    inputs: usize,
}

pub(super) fn compile(program: &RuleProgram) -> Result<NativeRule, Error> {
    let code = Emitter::lower(program)?;
    Ok(NativeRule {
        page: Page::publish(&code)?,
        inputs: usize::from(program.inputs),
    })
}

impl NativeRule {
    #[inline]
    pub(super) fn evaluate(&self, inputs: &[i64]) -> Result<i64, Error> {
        if inputs.len() != self.inputs {
            return Err(Error("wrong number of rule inputs".into()));
        }
        let mut scratch = [MaybeUninit::<i64>::uninit(); MAX_STACK];
        type Entry = unsafe extern "system" fn(*const i64, *mut MaybeUninit<i64>) -> u32;
        // SAFETY: The RX page begins with a generated Win64 leaf function of
        // this signature. Validated indices address only inputs and scratch.
        // The function never calls out, unwinds, or keeps either pointer.
        let status = unsafe {
            let entry: Entry = std::mem::transmute(self.page.0.as_ptr());
            entry(inputs.as_ptr(), scratch.as_mut_ptr())
        };
        if status != 0 {
            return Err(Error("rule arithmetic overflow or division by zero".into()));
        }
        // SAFETY: The only success exit has written the result to slot zero.
        Ok(unsafe { scratch[0].assume_init() })
    }
}

const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const SCRATCH: u8 = 9;
const STACK_REGISTERS: [u8; 3] = [8, 10, 11];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand {
    Register(u8),
    Memory(u8, u8),
}

impl Operand {
    fn slot(index: usize) -> Self {
        if let Some(&register) = STACK_REGISTERS.get(index) {
            Self::Register(register)
        } else {
            Self::Memory(
                SCRATCH,
                u8::try_from(index * 8).expect("validated stack slot"),
            )
        }
    }

    fn register_or_rax(self) -> u8 {
        match self {
            Self::Register(register) => register,
            Self::Memory(..) => RAX,
        }
    }
}

#[derive(Default)]
struct Emitter {
    bytes: Vec<u8>,
    failures: Vec<usize>,
}

impl Emitter {
    fn lower(program: &RuleProgram) -> Result<Vec<u8>, Error> {
        use Operand::{Memory, Register};
        let mut code = Self::default();
        code.mov(SCRATCH, Register(RDX));
        let mut depth = 0;
        let mut declines = Vec::new();
        for &op in &program.ops {
            match op {
                Op::Input(index) => {
                    code.assign(Operand::slot(depth), Memory(RCX, index * 8));
                    depth += 1;
                }
                Op::Const(value) => {
                    let destination = Operand::slot(depth);
                    let register = destination.register_or_rax();
                    code.immediate(register, value);
                    code.store(destination, register);
                    depth += 1;
                }
                Op::Binary(operation) => {
                    code.binary(
                        operation,
                        Operand::slot(depth - 2),
                        Operand::slot(depth - 1),
                    );
                    depth -= 1;
                }
                Op::Select => {
                    let destination = Operand::slot(depth - 3);
                    code.cmp_immediate(destination, 0);
                    code.mov(RAX, Operand::slot(depth - 2));
                    code.instruction(&[0x0f, 0x44], RAX, Operand::slot(depth - 1)); // cmove
                    code.store(destination, RAX);
                    depth -= 2;
                }
                Op::Guard => {
                    depth -= 1;
                    code.cmp_immediate(Operand::slot(depth), 0);
                    declines.push(code.branch(0x84)); // je
                }
            }
        }
        debug_assert_eq!(depth, 1);
        code.assign(Memory(SCRATCH, 0), Operand::slot(0));
        code.bytes.extend([0x31, 0xc0, 0xc3]); // xor eax,eax; ret
        let failure = code.bytes.len();
        code.bytes.extend([0xb8, 1, 0, 0, 0, 0xc3]); // mov eax,1; ret
        if !declines.is_empty() {
            let declined = code.bytes.len();
            code.bytes.extend([0x31, 0xc0]); // xor eax,eax
            code.store(Memory(SCRATCH, 0), RAX);
            code.bytes.push(0xc3); // ret
            for site in declines {
                code.patch(site, declined);
            }
        }
        if code.bytes.len() > CODE_LIMIT {
            return Err(Error("native rule code exceeds limit".into()));
        }
        for site in std::mem::take(&mut code.failures) {
            code.patch(site, failure);
        }
        Ok(code.bytes)
    }

    fn binary(&mut self, operation: Binary, left: Operand, right: Operand) {
        use Binary::*;
        if matches!(operation, F32Lt | F32Le | F32Add | F32Sub | F32Mul | F32Div) {
            self.bytes.push(0x66);
            self.instruction_width(&[0x0f, 0x6e], 0, left, false); // movd xmm0,left
            self.bytes.push(0x66);
            self.instruction_width(&[0x0f, 0x6e], 1, right, false); // movd xmm1,right
            match operation {
                F32Lt | F32Le => {
                    self.bytes.extend([0x0f, 0x2e, 0xc8]); // ucomiss xmm1,xmm0
                    // Reversing the operands makes both conditions false when unordered.
                    self.set_al(if operation == F32Lt { 0x97 } else { 0x93 });
                }
                _ => {
                    let opcode = match operation {
                        F32Add => 0x58,
                        F32Sub => 0x5c,
                        F32Mul => 0x59,
                        F32Div => 0x5e,
                        _ => unreachable!(),
                    };
                    self.bytes.extend([0xf3, 0x0f, opcode, 0xc1]); // scalar xmm0,xmm1
                    self.bytes.extend([0x66, 0x0f, 0x7e, 0xc0]); // movd eax,xmm0
                    self.bytes.extend([0x0f, 0x2e, 0xc0]); // ucomiss xmm0,xmm0
                    let ordered = self.branch(0x8b); // jnp
                    self.immediate(RAX, i64::from(super::CANONICAL_F32_NAN));
                    self.patch(ordered, self.bytes.len());
                }
            }
            self.store(left, RAX);
            return;
        }
        let register = match operation {
            Div | Rem | Lt | Eq => RAX,
            _ => left.register_or_rax(),
        };
        self.mov(register, left);
        match operation {
            Add | Sub | Mul => {
                let opcode: &[u8] = match operation {
                    Add => &[0x03],
                    Sub => &[0x2b],
                    _ => &[0x0f, 0xaf],
                };
                self.instruction(opcode, register, right);
                self.fail_if(0x80); // jo
            }
            BitAnd => self.instruction(&[0x23], register, right),
            Div | Rem => {
                self.cmp_immediate(right, 0);
                self.fail_if(0x84); // je
                self.cmp_immediate(right, -1);
                let safe = self.branch(0x85); // jne
                self.immediate(RDX, i64::MIN);
                self.instruction(&[0x3b], RAX, Operand::Register(RDX)); // cmp
                self.fail_if(0x84); // je
                self.patch(safe, self.bytes.len());
                self.bytes.extend([0x48, 0x99]); // cqo
                self.instruction(&[0xf7], 7, right); // idiv
            }
            Lt | Eq => {
                self.instruction(&[0x3b], RAX, right); // cmp
                self.set_al(if matches!(operation, Lt) { 0x9c } else { 0x94 });
            }
            Min | Max => {
                self.instruction(&[0x3b], register, right); // cmp
                let cmov = if matches!(operation, Min) { 0x4f } else { 0x4c };
                self.instruction(&[0x0f, cmov], register, right);
            }
            F32Lt | F32Le | F32Add | F32Sub | F32Mul | F32Div => unreachable!(),
        }
        self.store(
            left,
            if matches!(operation, Rem) {
                RDX
            } else {
                register
            },
        );
    }

    fn set_al(&mut self, condition: u8) {
        self.bytes.extend([0x0f, condition, 0xc0]); // setcc al
        self.bytes.extend([0x0f, 0xb6, 0xc0]); // movzx eax,al
    }

    fn assign(&mut self, destination: Operand, source: Operand) {
        let register = destination.register_or_rax();
        self.mov(register, source);
        self.store(destination, register);
    }

    fn mov(&mut self, register: u8, source: Operand) {
        if source != Operand::Register(register) {
            self.instruction(&[0x8b], register, source);
        }
    }

    fn store(&mut self, destination: Operand, register: u8) {
        if destination != Operand::Register(register) {
            self.instruction(&[0x89], register, destination);
        }
    }

    fn immediate(&mut self, register: u8, value: i64) {
        if let Ok(short) = u32::try_from(value) {
            if register >= 8 {
                self.bytes.push(0x41);
            }
            self.bytes.push(0xb8 + (register & 7));
            self.bytes.extend(short.to_le_bytes());
        } else {
            self.bytes
                .extend([0x48 | (register >> 3), 0xb8 + (register & 7)]);
            self.bytes.extend(value.to_le_bytes());
        }
    }

    fn cmp_immediate(&mut self, operand: Operand, value: i8) {
        self.instruction(&[0x83], 7, operand);
        self.bytes.extend(value.to_le_bytes());
    }

    fn instruction(&mut self, opcode: &[u8], register: u8, operand: Operand) {
        self.instruction_width(opcode, register, operand, true);
    }

    fn instruction_width(&mut self, opcode: &[u8], register: u8, operand: Operand, wide: bool) {
        let (base, mode) = match operand {
            Operand::Register(register) => (register, 0xc0),
            Operand::Memory(base, displacement) => {
                (base, if displacement <= 127 { 0x40 } else { 0x80 })
            }
        };
        self.bytes
            .push(0x40 | (u8::from(wide) << 3) | ((register >> 3) << 2) | (base >> 3));
        self.bytes.extend(opcode);
        self.bytes.push(mode | ((register & 7) << 3) | (base & 7));
        if let Operand::Memory(_, displacement) = operand {
            if mode == 0x40 {
                self.bytes.push(displacement);
            } else {
                self.bytes.extend(u32::from(displacement).to_le_bytes());
            }
        }
    }

    fn branch(&mut self, condition: u8) -> usize {
        self.bytes.extend([0x0f, condition]);
        let site = self.bytes.len();
        self.bytes.extend([0; 4]);
        site
    }

    fn fail_if(&mut self, condition: u8) {
        let site = self.branch(condition);
        self.failures.push(site);
    }

    fn patch(&mut self, site: usize, target: usize) {
        let offset = i32::try_from(target - (site + 4)).expect("bounded forward native branch");
        self.bytes[site..site + 4].copy_from_slice(&offset.to_le_bytes());
    }
}
