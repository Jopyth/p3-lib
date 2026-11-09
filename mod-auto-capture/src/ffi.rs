//! Lets the player's ships capture instead of plunder in sea battles the player doesn't steer.
//!
//! After a won boarding fight the game asks `0x006212FB` (thiscall on the boarding ship, args
//! `defender, check`) whether the ship is captured (1) or only plundered (0). For ships of a human
//! merchant (`battle+0x5D[slot]` is 0) the answer is bit 0x80 of the ship's battle flags, which only
//! the capture order of a manually steered battle sets (op handler `0x00541380`). In automatic
//! battles nobody sets it, so the player always plunders.
//!
//! The mod replaces that answer: if the flag isn't set, the call is the final decision
//! (`check != 0`, the AI's "should I board" query stays unchanged) and the battle isn't the one shown
//! on screen, the ship captures.

use std::{arch::global_asm, ffi::c_void};

use hooklet::windows::x86::{deploy_rel32_raw, replace_slice_rwx, X86Rel32Type};
use log::{debug, error};
use p3_api::mods;

/// `mov al, byte ptr [esi+0x12A]` in `0x006212FB`, reached for ships of human merchants. It's followed
/// by `pop edi; pop esi; pop ebp; shr al, 7; ret 8`.
const HUMAN_CAPTURE_FLAG_ADDRESS: u32 = 0x006214A2;
const HUMAN_CAPTURE_FLAG_ORIGINAL: [u8; 15] = [0x8a, 0x86, 0x2a, 0x01, 0x00, 0x00, 0x5f, 0x5e, 0x5d, 0xc0, 0xe8, 0x07, 0xc2, 0x08, 0x00];

/// Index of the sea battle shown on screen (manual steering), 0xFF if none.
const SHOWN_BATTLE_ADDRESS: u32 = 0x006E59CC;

const SHIP_FLAGS_OFFSET: usize = 0x12A;
const SHIP_BATTLE_OFFSET: usize = 0x12C;
const BATTLE_INDEX_OFFSET: usize = 0x662;
const FLAG_CAPTURE: u8 = 0x80;

static mut SHOWN_BATTLE: u32 = SHOWN_BATTLE_ADDRESS;

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    mods::init_mod();

    if !p3_api::version::is_supported() {
        return p3_api::version::UNSUPPORTED_BUILD;
    }
    SHOWN_BATTLE = p3_api::addr(SHOWN_BATTLE);

    let patch_address = p3_api::addr(HUMAN_CAPTURE_FLAG_ADDRESS);
    let original = std::slice::from_raw_parts(patch_address as *const u8, HUMAN_CAPTURE_FLAG_ORIGINAL.len());
    if original != HUMAN_CAPTURE_FLAG_ORIGINAL {
        error!("Unexpected bytes {original:02x?} at {patch_address:#x}, is this the right exe?");
        return 1;
    }

    debug!("Capture instead of plunder in automatic sea battles ({patch_address:#x})");
    if deploy_rel32_raw(patch_address, (&human_capture_decision_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        return 2;
    }
    if replace_slice_rwx(patch_address + 5, &[0x90]).is_err() {
        return 3;
    }

    0
}

#[no_mangle]
unsafe extern "C" fn human_capture_decision(ship: *const u8, check: u32) -> u32 {
    let flags = *ship.add(SHIP_FLAGS_OFFSET);
    if flags & FLAG_CAPTURE != 0 {
        return 1;
    }
    if check == 0 {
        return 0;
    }
    let battle = *(ship.add(SHIP_BATTLE_OFFSET) as *const *const u8);
    let battle_index = *(battle.add(BATTLE_INDEX_OFFSET) as *const u16);
    let shown_battle = *(SHOWN_BATTLE as *const u8);
    if shown_battle as u16 == battle_index {
        debug!("Battle {battle_index} is steered manually, keeping the player's choice (plunder)");
        return 0;
    }
    debug!("Automatic battle {battle_index}: capturing instead of plundering");
    1
}

extern "C" {
    static human_capture_decision_detour: c_void;
}

// Stack at the patch address: edi, esi, ebp, return address, defender, check.
global_asm!("
.global {detour}
{detour}:
push [esp+0x14]
push esi
call {decision}
add esp, 8
pop edi
pop esi
pop ebp
ret 8
",
detour = sym human_capture_decision_detour,
decision = sym human_capture_decision);
