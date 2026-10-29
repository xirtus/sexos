//! Historical v1 register-only fault codec, retained for regression coverage.
//! Active recovery uses `paging::FaultNotice` with unique pending tokens.
//! This old codec confers no mapping or resume authority.
//!
//! Slot 2 is a boot-granted Domain capability. The kernel reports the fault
//! through that capability; the receiver must also validate the authenticated
//! `caller_pd` against the reported domain. No pointer or lent capability is
//! carried here. This protocol does not yet define request identity/completion.

/// Reserved fault notification opcode on the sext capability edge. Zero remains
/// the PDX listen empty sentinel. This value is not a syscall number.
pub const OP_PAGE_FAULT: u64 = 0x100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFaultError {
    WrongOpcode,
    NonCanonicalAddress,
    InvalidDomain,
    ErrorCodeOverflow,
    CallerMismatch,
}

/// All values have stable lifetimes when copied into the destination ring.
/// Validation describes transport shape only, never permission to map a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFaultRequest {
    fault_addr: u64,
    error_code: u32,
    pd_id: u32,
}

/// Current x86-64 configuration uses four-level (48-bit) virtual addressing.
/// Both canonical halves and address zero are representable fault reports.
pub const fn is_canonical_fault_address(addr: u64) -> bool {
    addr <= 0x0000_7fff_ffff_ffff || addr >= 0xffff_8000_0000_0000
}

impl PageFaultRequest {
    /// Domain zero is the kernel and must not be forwarded to a user pager.
    /// Kernel registry bounds and actual capability authority are checked by
    /// the sender separately; this portable codec only enforces the u32 ABI.
    pub const fn new(fault_addr: u64, error_code: u32, pd_id: u64) -> Result<Self, PageFaultError> {
        if !is_canonical_fault_address(fault_addr) {
            return Err(PageFaultError::NonCanonicalAddress);
        }
        if pd_id == 0 || pd_id > u32::MAX as u64 {
            return Err(PageFaultError::InvalidDomain);
        }
        Ok(Self {
            fault_addr,
            error_code,
            pd_id: pd_id as u32,
        })
    }

    /// Listen registers: rax=OP_PAGE_FAULT, rsi=authenticated caller_pd,
    /// rdx=fault_addr, r10=zero-extended error_code, r8=pd_id.
    pub const fn decode(
        opcode: u64,
        caller_pd: u64,
        args: [u64; 3],
    ) -> Result<Self, PageFaultError> {
        if opcode != OP_PAGE_FAULT {
            return Err(PageFaultError::WrongOpcode);
        }
        if args[1] > u32::MAX as u64 {
            return Err(PageFaultError::ErrorCodeOverflow);
        }
        let request = match Self::new(args[0], args[1] as u32, args[2]) {
            Ok(request) => request,
            Err(error) => return Err(error),
        };
        if caller_pd != request.pd_id as u64 {
            return Err(PageFaultError::CallerMismatch);
        }
        Ok(request)
    }

    pub const fn args(self) -> [u64; 3] {
        [self.fault_addr, self.error_code as u64, self.pd_id as u64]
    }

    pub const fn fault_addr(self) -> u64 {
        self.fault_addr
    }
    pub const fn error_code(self) -> u32 {
        self.error_code
    }
    pub const fn pd_id(self) -> u32 {
        self.pd_id
    }
}
