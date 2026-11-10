//! Lets the player's ships capture instead of plunder in sea battles the player doesn't steer.
//!
//! After a won boarding fight the game asks `0x006212FB` (thiscall on the boarding ship, args
//! `defender, check`) whether the ship is captured (1) or only plundered (0). `check = 0` is the AI's
//! "should I board" query on contact, `check = 1` the final decision. Two branches are patched:
//!
//! - Slots of a human merchant (`battle+0x5D[slot]` is 0): the answer is bit 0x80 of the ship's battle
//!   flags, the capture order of a manually steered battle (op handler `0x00541380`). If it isn't set,
//!   the decision is final and the battle isn't the one shown on screen, the ship captures.
//! - AI-steered slots: after the sailor check (sailors plus cutlasses at least twice the defender's
//!   sailors), every attacker with an owner plunders (`0x0062140E`), only pirates without owner roll
//!   for a capture. The mod lets ships of the player capture there. An automatic battle of the player
//!   apparently runs the player's slot as an AI slot, so this is the branch that matters.
//!
//! Only ships with `CAPTURE` (upper case) in their name capture. All other ships behave as before.

use std::{arch::global_asm, ffi::c_void};

use hooklet::windows::x86::{deploy_rel32_raw, replace_slice_rwx, X86Rel32Type};
use log::{debug, error};
use p3_api::{mods, ships::ShipsPtr};

/// `mov al, byte ptr [esi+0x12A]` in `0x006212FB`, reached for ships of human merchants. It's followed
/// by `pop edi; pop esi; pop ebp; shr al, 7; ret 8`.
const HUMAN_CAPTURE_FLAG_ADDRESS: u32 = 0x006214A2;
const HUMAN_CAPTURE_FLAG_ORIGINAL: [u8; 15] = [0x8a, 0x86, 0x2a, 0x01, 0x00, 0x00, 0x5f, 0x5e, 0x5d, 0xc0, 0xe8, 0x07, 0xc2, 0x08, 0x00];

/// `cmp word ptr [eax+edx*2+0x35A], bp; jne 0x006213B0` (bp = 0xFF): AI slot, attacker has an owner, so plunder.
const AI_OWNER_CHECK_ADDRESS: u32 = 0x0062140E;
const AI_OWNER_CHECK_ORIGINAL: [u8; 10] = [0x66, 0x39, 0xac, 0x50, 0x5a, 0x03, 0x00, 0x00, 0x75, 0x98];
/// Next instruction: pirates without owner roll for a capture.
static mut AI_OWNER_CHECK_CONTINUATION: u32 = 0x00621418;
/// `pop edi; pop esi; mov al, 1; pop ebp; ret 8`
static mut RETURN_CAPTURE: u32 = 0x00621348;
/// `pop edi; pop esi; xor al, al; pop ebp; ret 8`
static mut RETURN_PLUNDER: u32 = 0x006213B0;

/// Index of the sea battle shown on screen (manual steering), 0xFF if none.
static mut SHOWN_BATTLE: u32 = 0x006E59CC;
/// Merchant index of the local human player (u32).
static mut PLAYER_MERCHANT: u32 = 0x006DFC14;

/// Ships of the player capture only with this text in their name (case-sensitive).
const CAPTURE_NAME_MARKER: &str = "CAPTURE";

const SHIP_INDEX_OFFSET: usize = 0x4;
const SHIP_FLAGS_OFFSET: usize = 0x12A;
const SHIP_BATTLE_OFFSET: usize = 0x12C;
const BATTLE_INDEX_OFFSET: usize = 0x662;
const FLAG_CAPTURE: u8 = 0x80;

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    mods::init_mod();

    if !p3_api::version::is_supported() {
        return p3_api::version::UNSUPPORTED_BUILD;
    }
    SHOWN_BATTLE = p3_api::addr(SHOWN_BATTLE);
    PLAYER_MERCHANT = p3_api::addr(PLAYER_MERCHANT);
    AI_OWNER_CHECK_CONTINUATION = p3_api::addr(AI_OWNER_CHECK_CONTINUATION);
    RETURN_CAPTURE = p3_api::addr(RETURN_CAPTURE);
    RETURN_PLUNDER = p3_api::addr(RETURN_PLUNDER);

    let human_address = p3_api::addr(HUMAN_CAPTURE_FLAG_ADDRESS);
    let ai_address = p3_api::addr(AI_OWNER_CHECK_ADDRESS);
    if !check_bytes(human_address, &HUMAN_CAPTURE_FLAG_ORIGINAL) || !check_bytes(ai_address, &AI_OWNER_CHECK_ORIGINAL) {
        return 1;
    }

    debug!("Capture instead of plunder in automatic sea battles ({human_address:#x}, {ai_address:#x})");
    if deploy_rel32_raw(human_address, (&human_capture_decision_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        return 2;
    }
    if replace_slice_rwx(human_address + 5, &[0x90]).is_err() {
        return 3;
    }
    if deploy_rel32_raw(ai_address, (&ai_owner_check_detour) as *const _ as _, X86Rel32Type::Jump).is_err() {
        return 4;
    }
    if replace_slice_rwx(ai_address + 5, &[0x90; 5]).is_err() {
        return 5;
    }

    0
}

unsafe fn check_bytes(address: u32, expected: &[u8]) -> bool {
    let actual = std::slice::from_raw_parts(address as *const u8, expected.len());
    if actual != expected {
        error!("Unexpected bytes {actual:02x?} at {address:#x}, is this the right exe?");
        return false;
    }
    true
}

unsafe fn battle_index(ship: *const u8) -> u16 {
    let battle = *(ship.add(SHIP_BATTLE_OFFSET) as *const *const u8);
    *(battle.add(BATTLE_INDEX_OFFSET) as *const u16)
}

/// The ship's name if it contains the capture marker.
unsafe fn capture_ship_name(ship: *const u8) -> Option<String> {
    let ship_index = *(ship.add(SHIP_INDEX_OFFSET) as *const u32);
    let name = ShipsPtr::new().get_ship(ship_index as u16)?.get_name();
    name.contains(CAPTURE_NAME_MARKER).then_some(name)
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
    let battle_index = battle_index(ship);
    let shown_battle = *(SHOWN_BATTLE as *const u8);
    if shown_battle as u16 == battle_index {
        debug!("Human slot, battle {battle_index} is steered manually: keeping the player's choice (plunder)");
        return 0;
    }
    match capture_ship_name(ship) {
        Some(name) => {
            debug!("Human slot, automatic battle {battle_index}: \"{name}\" captures instead of plundering");
            1
        }
        None => {
            debug!("Human slot, automatic battle {battle_index}: no {CAPTURE_NAME_MARKER} in the ship's name, plundering");
            0
        }
    }
}

#[no_mangle]
unsafe extern "C" fn ai_owner_capture_decision(ship: *const u8, owner: u32, check: u32) -> u32 {
    let player = *(PLAYER_MERCHANT as *const u32);
    if owner != player {
        if check != 0 {
            debug!("AI slot, battle {}, owner {owner} (player {player}): plundering as usual", battle_index(ship));
        }
        return 0;
    }
    let name = capture_ship_name(ship);
    if check != 0 {
        let battle_index = battle_index(ship);
        match &name {
            Some(name) => debug!("AI slot of the player, battle {battle_index}: \"{name}\" captures instead of plundering"),
            None => debug!("AI slot of the player, battle {battle_index}: no {CAPTURE_NAME_MARKER} in the ship's name, plundering"),
        }
    }
    name.is_some() as u32
}

extern "C" {
    static human_capture_decision_detour: c_void;
    static ai_owner_check_detour: c_void;
}

// Stack at both patch addresses: edi, esi, ebp, return address, defender, check. esi is the attacker.
global_asm!("
.global {human_detour}
{human_detour}:
push [esp+0x14]
push esi
call {human_decision}
add esp, 8
pop edi
pop esi
pop ebp
ret 8

.global {ai_detour}
{ai_detour}:
cmp word ptr [eax+edx*2+0x35A], bp
jne 2f
jmp [{continuation}]
2:
movzx edx, word ptr [eax+edx*2+0x35A]
push [esp+0x14]
push edx
push esi
call {ai_decision}
add esp, 12
test eax, eax
jz 3f
jmp [{capture}]
3:
jmp [{plunder}]
",
human_detour = sym human_capture_decision_detour,
human_decision = sym human_capture_decision,
ai_detour = sym ai_owner_check_detour,
ai_decision = sym ai_owner_capture_decision,
continuation = sym AI_OWNER_CHECK_CONTINUATION,
capture = sym RETURN_CAPTURE,
plunder = sym RETURN_PLUNDER);
