use std::mem;
use std::sync::atomic::{AtomicPtr, Ordering};

use hooklet::windows::x86::{hook_call_rel32, CallRel32Hook};
use log::{debug, error};
use p3_api::version::{addr, is_supported};
use windows::core::PSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

const IMAGE_BASE: u32 = 0x0040_0000;
const WINMAIN_ADDRESS: u32 = 0x0064BE10;
/// The `call WinMain` in the CRT startup code.
const WINMAIN_CALL_ADDRESS: u32 = 0x0063CA22;
static HOOK_PTR: AtomicPtr<CallRel32Hook> = AtomicPtr::new(std::ptr::null_mut());

#[no_mangle]
extern "system" fn DllMain(_hist_dll: *const u8, fdw_reason: u32, _lpv_reserved: *const u8) -> u32 {
    if fdw_reason == DLL_PROCESS_ATTACH {
        let _ = log::set_logger(&win_dbg_logger::DEBUGGER_LOGGER);
        log::set_max_level(log::LevelFilter::Trace);
        if !is_supported() {
            error!("Unsupported game build, not loading any mods");
            return 1;
        }
        debug!("DllMain patching WinMain call");
        unsafe {
            let call_address = addr(WINMAIN_CALL_ADDRESS);
            let call = std::slice::from_raw_parts(call_address as *const u8, 5);
            let target = (call_address + 5).wrapping_add(u32::from_le_bytes(call[1..5].try_into().unwrap()));
            if call[0] != 0xE8 || target != addr(WINMAIN_ADDRESS) {
                error!("Unexpected bytes {call:02x?} at {call_address:#x}, expected a call to WinMain");
                return 1;
            }
            let hook = match hook_call_rel32(call_address - IMAGE_BASE, WinMain_hook as usize as u32) {
                Ok(hook) => hook,
                Err(e) => {
                    error!("Failed to set hook: {:?}", e);
                    return 1;
                }
            };
            debug!("Storing hook");
            HOOK_PTR.store(Box::into_raw(Box::new(hook)), Ordering::SeqCst);
        }
        debug!("DllMain finished successfully")
    }
    1
}

#[no_mangle]
extern "stdcall" fn WinMain_hook(h_instance: HMODULE, h_prev_instance: HMODULE, lp_cmd_line: PSTR, n_show_cmd: u32) -> i32 {
    debug!("WinMain_hook starting");
    let original_winmain: extern "stdcall" fn(h_instance: HMODULE, h_prev_instance: HMODULE, lp_cmd_line: PSTR, n_show_cmd: u32) -> i32 =
        unsafe { mem::transmute(addr(WINMAIN_ADDRESS)) };
    crate::load();
    debug!("WinMain_hook calling original main");
    original_winmain(h_instance, h_prev_instance, lp_cmd_line, n_show_cmd)
}
