//! Bounded Linux GNU x86-64 code publication and System V unwind registration.
//!
//! This module accepts only internally generated, verified Cranelift code. It
//! does not provide isolation for arbitrary native bytes. There are no external
//! calls or relocations in the admitted generated functions. Published code and
//! its `.eh_frame` are immutable, and all invocations borrow their common owner.
use crate::Error;
use cranelift_codegen::isa::{TargetIsa, unwind::systemv::UnwindInfo};
use gimli::write::{Address, EhFrame, EndianVec, FrameTable, Writer};
use std::ffi::{c_int, c_ulong, c_void};
use std::ptr::NonNull;

const CODE_LIMIT: usize = 16 * 1024;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const PROT_EXEC: c_int = 4;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;
const READ_IMPLIES_EXEC: c_int = 0x0040_0000;

unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, length: usize, prot: c_int) -> c_int;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn personality(persona: c_ulong) -> c_int;
}

#[link(name = "gcc_s")]
unsafe extern "C" {
    fn __register_frame(begin: *const c_void);
    fn __deregister_frame(begin: *const c_void);
}

#[derive(Debug)]
struct Mapping(NonNull<c_void>);

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: This object exclusively owns one successful CODE_LIMIT-byte
        // mmap. It cannot be cloned and no invocation borrows it during drop.
        unsafe { munmap(self.0.as_ptr(), CODE_LIMIT) };
    }
}

#[derive(Debug)]
pub(super) struct RegisteredPage {
    mapping: Mapping,
    unwind_offset: usize,
}

// SAFETY: Publication removes write permission before returning this object.
// Shared calls borrow the immutable mapping and use disjoint caller scratch.
// Drop deregisters CFI before the exclusive mapping owner unmaps its bytes.
unsafe impl Send for RegisteredPage {}
// SAFETY: No safe interface exposes mutation or retains argument pointers.
unsafe impl Sync for RegisteredPage {}

impl RegisteredPage {
    pub(super) fn publish(
        code: &[u8],
        unwind: UnwindInfo,
        target: &dyn TargetIsa,
    ) -> Result<Self, Error> {
        if code.is_empty() || code.len() > CODE_LIMIT {
            return Err(Error("invalid Linux native code size".into()));
        }
        // READ_IMPLIES_EXEC can turn PROT_READ|PROT_WRITE into an executable
        // mapping. Refuse that personality instead of claiming W^X from the
        // requested flags alone. Query failure also declines publication.
        // SAFETY: 0xffff_ffff queries the current personality without changing it.
        let current = unsafe { personality(c_ulong::from(u32::MAX)) };
        if current < 0 {
            return Err(platform_error("personality query"));
        }
        if current & READ_IMPLIES_EXEC != 0 {
            return Err(Error(
                "Linux native W^X requires READ_IMPLIES_EXEC disabled".into(),
            ));
        }
        // SAFETY: Request a bounded anonymous private mapping, writable only
        // during preparation. Linux mmap returns MAP_FAILED, not null, on error.
        let pointer = unsafe {
            mmap(
                std::ptr::null_mut(),
                CODE_LIMIT,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer == usize::MAX as *mut c_void {
            return Err(platform_error("mmap"));
        }
        let Some(pointer) = NonNull::new(pointer) else {
            // A null mapping is theoretically possible when mmap_min_addr is
            // zero. Release it explicitly because it cannot be owned by NonNull.
            // SAFETY: The call above succeeded and returned this mapping.
            unsafe { munmap(pointer, CODE_LIMIT) };
            return Err(Error("Linux native mapping returned a null address".into()));
        };
        let mapping = Mapping(pointer);
        let mut table = FrameTable::default();
        let mut cie = target
            .create_systemv_cie()
            .ok_or_else(|| Error("missing System V CIE".into()))?;
        // Absolute addresses need no relocation writer or PC-relative base.
        cie.fde_address_encoding = gimli::DW_EH_PE_absptr;
        let cie = table.add_cie(cie);
        table.add_fde(
            cie,
            unwind.to_fde(Address::Constant(pointer.as_ptr() as u64)),
        );
        let mut frame = EhFrame(EndianVec::new(gimli::LittleEndian));
        table
            .write_eh_frame(&mut frame)
            .map_err(|e| Error(format!("Linux native CFI: {e}")))?;
        frame
            .0
            .write_u32(0)
            .map_err(|e| Error(format!("Linux native CFI terminator: {e}")))?;
        let frame = frame.0.into_vec();
        let unwind_offset = code.len().next_multiple_of(8);
        if unwind_offset > CODE_LIMIT || frame.len() > CODE_LIMIT - unwind_offset {
            return Err(Error(
                "Linux native code and unwind metadata exceed limit".into(),
            ));
        }
        // SAFETY: Checked non-overlapping destination ranges are inside the
        // private writable mapping. They remain unexecutable while being copied.
        unsafe {
            std::ptr::copy_nonoverlapping(code.as_ptr(), pointer.as_ptr().cast::<u8>(), code.len());
            std::ptr::copy_nonoverlapping(
                frame.as_ptr(),
                pointer.as_ptr().cast::<u8>().add(unwind_offset),
                frame.len(),
            );
            if mprotect(pointer.as_ptr(), CODE_LIMIT, PROT_READ | PROT_EXEC) != 0 {
                return Err(platform_error("mprotect"));
            }
        }
        // x86-64 has coherent instruction/data caches. mprotect completes the
        // protection transition before a pointer can escape this function. The
        // CFI table resides in the same immutable allocation as the code.
        let page = Self {
            mapping,
            unwind_offset,
        };
        // SAFETY: Gimli emitted complete, zero-terminated .eh_frame records for
        // this exact code address. Both code and CFI remain live until Drop.
        unsafe { __register_frame(page.unwind()) };
        Ok(page)
    }

    pub(super) fn entry(&self) -> *const c_void {
        self.mapping.0.as_ptr()
    }

    fn unwind(&self) -> *const c_void {
        // SAFETY: The offset was checked against the owned mapping at publication.
        unsafe {
            self.mapping
                .0
                .as_ptr()
                .cast::<u8>()
                .add(self.unwind_offset)
                .cast()
        }
    }
}

impl Drop for RegisteredPage {
    fn drop(&mut self) {
        // SAFETY: No invocation borrows this final owner. Unregister before the
        // Mapping field releases both code and CFI storage.
        unsafe { __deregister_frame(self.unwind()) };
    }
}

fn platform_error(operation: &str) -> Error {
    Error(format!(
        "Linux native {operation}: {}",
        std::io::Error::last_os_error()
    ))
}
