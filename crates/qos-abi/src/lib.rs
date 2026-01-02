#![no_std]

extern crate alloc;

use alloc::{string::String, vec::Vec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// ABI version for syscall/message compatibility.
pub const ABI_VERSION: u32 = 1;

/// Shared-memory syscall ABI (Ring3 <-> kernel).
///
/// This is a small, allocation-free wire format intended for bare-metal.
/// Higher-level `QosRequest/QosResponse` can be layered on top for hosted JSON.
pub mod shm {
    /// Shared-memory ABI version. Must match `ABI_VERSION`.
    pub const SHM_ABI_VERSION: u32 = super::ABI_VERSION;

    pub const STATUS_OK: u32 = 0;
    pub const STATUS_ERR: u32 = 1;

    pub const OP_GET_ABI_VERSION: u32 = 0;
    pub const OP_SUBMIT_BELL: u32 = 1;
    pub const OP_GET_STATUS: u32 = 2;
    pub const OP_GET_RESULT: u32 = 3;
    pub const OP_EXIT: u32 = 4;
    pub const OP_DISPATCH_NEXT: u32 = 5;
    pub const OP_CANCEL: u32 = 6;
    pub const OP_SUBMIT_IR: u32 = 7;

    // VFS (filesystem) operations via a header+payload buffer.
    // The user passes arg0=ptr_to_buffer, arg1=total_bytes.
    pub const OP_VFS_IO: u32 = 8;

    // IR formats for OP_SUBMIT_IR (arg2 would exist in a larger call frame; for now
    // we assume QASM2, but keep constants for forward-compat).
    pub const IRFMT_QASM2: u32 = 1;

    // Versioned submit header written in user memory. Kernel copies and validates it.
    pub const SUBMIT_HDR_VERSION: u32 = 1;

    pub const VFS_HDR_VERSION: u32 = 1;
    pub const VFS_OP_LIST_DIR: u32 = 1;
    pub const VFS_OP_READ: u32 = 2;
    pub const VFS_OP_WRITE: u32 = 3;
    pub const VFS_OP_REMOVE: u32 = 4;

    /// Header for `OP_VFS_IO`.
    ///
    /// Buffer layout in user memory:
    /// - `ShmVfsIoHeader`
    /// - `path_len` bytes of UTF-8-ish path (e.g. "/ram/foo.qasm")
    /// - `data_cap` bytes of data region
    ///
    /// For `VFS_OP_WRITE`, the data region contains `data_len` input bytes.
    /// For `VFS_OP_READ`/`VFS_OP_LIST_DIR`, the kernel writes up to `data_cap` bytes and
    /// sets `data_len` to bytes written.
    #[repr(C)]
    pub struct ShmVfsIoHeader {
        pub version: u32,
        pub vfs_op: u32,
        pub path_len: u32,
        pub data_len: u32,
        pub data_cap: u32,
        pub _reserved: u32,
    }

    /// Submit header for `OP_SUBMIT_IR`.
    ///
    /// The user passes a pointer to this header in `ShmCall.arg0`, and the total byte size
    /// (header + payload) in `ShmCall.arg1`.
    ///
    /// The QASM payload bytes immediately follow this header in memory.
    #[repr(C)]
    pub struct ShmSubmitIrHeader {
        pub version: u32,
        pub ir_format: u32,
        pub n_qubits: u32,
        pub shots: u32,
        pub payload_len: u32,
        pub _reserved: u32,
    }

    /// Fixed layout for a single syscall “call frame” in user-mapped memory.
    ///
    /// Field offsets (bytes):
    /// - abi_version: 0x00
    /// - op:          0x04
    /// - status:      0x08
    /// - ret0:        0x10
    /// - ret1:        0x18
    /// - arg0:        0x20
    /// - arg1:        0x28
    #[repr(C)]
    pub struct ShmCall {
        pub abi_version: u32,
        pub op: u32,
        pub status: u32,
        pub _reserved: u32,
        pub ret0: u64,
        pub ret1: u64,
        pub arg0: u64,
        pub arg1: u64,
    }
}

/// Kernel/userland handle (stable, small). Host services can map this to UUIDs.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobHandle(pub u64);

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrFormat {
    OpenQasm2,
    OpenQasm3,
    JsonIrV1,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ResultStatus {
    Ok,
    Error,
}

/// Minimal proc spec that a syscall/userland boundary can carry.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcSpec {
    pub name: String,
    pub ir_format: IrFormat,
    pub ir_bytes: Vec<u8>,
    pub n_qubits: u32,
    pub shots: u32,
}

/// Minimal result that the kernel/userland boundary can carry.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult {
    pub status: ResultStatus,
    pub counts_json: String,
    pub meta: String,
    pub error: Option<String>,
}

/// Syscall-like request messages.
///
/// This is intentionally small and no_std-friendly. The hosted HTTP API can be a superset.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QosRequest {
    Submit { proc: ProcSpec },
    Status { handle: JobHandle },
    GetResult { handle: JobHandle },
    DispatchNext,
    Cancel { handle: JobHandle },
    FinishOk { handle: JobHandle, result: JobResult },
    FinishErr { handle: JobHandle, error: String },
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QosResponse {
    SubmitOk { handle: JobHandle, state: JobState },
    StatusOk { handle: JobHandle, state: JobState },
    ResultOk { handle: JobHandle, result: JobResult },
    DispatchOk { dispatched: Option<JobHandle> },
    Ok,
    Err { message: String },
}
