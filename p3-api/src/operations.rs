use std::{ffi::c_void, mem::transmute};

use log::debug;

use crate::{data::p3_ptr::P3Pointer, operation::Operation};

pub const OPERATIONS_PTR: OperationsPtr = OperationsPtr::new();
const OPERATIONS_ADDRESS: u32 = 0x006DF2F0;
const EXECUTE_OPERATION_ADDRESS: u32 = 0x00535760;
const ENQUEUE_OPERATION_ADDRESS: u32 = 0x0054AA70;
const TRANSFER_LOADED_TRADEROUTE_ADDRESS: u32 = 0x005492D0;

#[derive(Clone, Debug, Copy)]
pub struct OperationsPtr {
    /// The address in the international exe. Use `get_address()`, which translates it to the running build.
    pub address: u32,
}

impl Default for OperationsPtr {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationsPtr {
    pub const fn new() -> Self {
        Self { address: OPERATIONS_ADDRESS }
    }

    pub unsafe fn get_status(&self) -> u32 {
        self.get(0x00)
    }

    pub unsafe fn set_current_operations_array_pos(&self, pos: u16) {
        self.set(0x474 + 0x0e, &pos)
    }

    pub unsafe fn get_player_merchant_index(&self) -> i32 {
        self.get(0x0924)
    }

    pub unsafe fn get_unpacked_traderoute_ptr(&self) -> *mut c_void {
        self.get(0x930)
    }

    pub unsafe fn enqueue_operation(&self, op: Operation) {
        debug!("Enqueuing operation {op:?}");
        let op_bytes = op.to_raw();
        let enqueue_operation: extern "thiscall" fn(*mut c_void, *const c_void) = transmute(crate::addr(ENQUEUE_OPERATION_ADDRESS));
        enqueue_operation(self.get_address() as _, op_bytes.as_ptr() as _)
    }

    pub unsafe fn transfer_loaded_traderoute(&self) {
        debug!("Transfer loaded traderoute");
        let transfer_loaded_traderoute: extern "thiscall" fn(*mut c_void) = transmute(crate::addr(TRANSFER_LOADED_TRADEROUTE_ADDRESS));
        transfer_loaded_traderoute(self.get_address() as _)
    }
}

impl P3Pointer for OperationsPtr {
    fn get_address(&self) -> u32 {
        crate::addr(self.address)
    }
}

pub fn execute_operation(op: &Operation) {
    debug!("execute_operation({:?})", op);
    let op = op.to_raw();
    let execute_operation: extern "thiscall" fn(op: *const u8) = unsafe { transmute(crate::addr(EXECUTE_OPERATION_ADDRESS)) };
    execute_operation(op.as_ptr());
}
