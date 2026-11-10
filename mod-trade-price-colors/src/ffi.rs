//! Colors good prices in the trade window.
//!
//! The trade window's row loop (`0x4A79BE`..`0x4A7D40`, `esi` = window, `ebx` = ware index)
//! formats the town's buy price `[esi + ware*4 + 0x5EC]` and sell price `[esi + ware*4 + 0x63C]`
//! and sets them as the text of the row's two price buttons with `0x4C7780` (thiscall
//! `set_text(text)`). The buttons draw their text in one of three colors depending on their
//! state (`0x4C769E`: widget `+0xC0`, `+0xC4`, `+0xC8`).
//!
//! The mod replaces both calls to `set_text` with a stub that calls it and then sets the
//! button's text colors: the configured color if the price is good, otherwise the button's
//! original colors.

use std::{arch::global_asm, collections::HashMap, ffi::c_void, sync::Mutex};

use hooklet::windows::x86::{deploy_rel32_raw, X86Rel32Type};
use log::{debug, info};

use crate::config::Config;

/// `call set_text` for the buy price button.
const BUY_SET_TEXT_CALL: u32 = 0x004A7B29;
/// `call set_text` for the sell price button.
const SELL_SET_TEXT_CALL: u32 = 0x004A7B59;
/// Price button `set_text`, thiscall, `ret 4`.
static mut SET_TEXT: u32 = 0x004C7780;

const BUY_PRICES_OFFSET: u32 = 0x5EC;
const SELL_PRICES_OFFSET: u32 = 0x63C;
/// The button's text colors for its states.
const COLOR_OFFSETS: [u32; 3] = [0xC0, 0xC4, 0xC8];

struct State {
    config: Config,
    /// The original colors of every price button seen, by button address.
    original_colors: HashMap<u32, [u32; 3]>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    p3_api::mods::init_mod();
    if !p3_api::version::is_supported() {
        return p3_api::version::UNSUPPORTED_BUILD;
    }
    SET_TEXT = p3_api::addr(SET_TEXT);

    let buy_call = p3_api::addr(BUY_SET_TEXT_CALL);
    let sell_call = p3_api::addr(SELL_SET_TEXT_CALL);
    for call in [buy_call, sell_call] {
        if !is_call_to(call, SET_TEXT) {
            log::error!("Unexpected code at {call:#x}, not patching");
            return 1;
        }
    }

    *STATE.lock().unwrap() = Some(State {
        config: Config::load(),
        original_colors: HashMap::new(),
    });

    debug!("Hooking the trade window's price buttons at {buy_call:#x} and {sell_call:#x}");
    if deploy_rel32_raw(buy_call, (&buy_set_text_stub) as *const _ as _, X86Rel32Type::Call).is_err() {
        return 2;
    }
    if deploy_rel32_raw(sell_call, (&sell_set_text_stub) as *const _ as _, X86Rel32Type::Call).is_err() {
        return 3;
    }
    info!("Trade price colors active");
    0
}

/// Whether `address` holds `call target` (E8 rel32).
unsafe fn is_call_to(address: u32, target: u32) -> bool {
    let code = address as *const u8;
    if *code != 0xE8 {
        return false;
    }
    let rel = (code.add(1) as *const i32).read_unaligned();
    address.wrapping_add(5).wrapping_add(rel as u32) == target
}

/// Called after the game set the text of a price button.
#[no_mangle]
unsafe extern "C" fn color_price_button(button: u32, window: u32, ware: u32, sell: u32) {
    if button == 0 || window == 0 {
        return;
    }
    let mut guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(state) = guard.as_mut() else { return };
    state.config.reload_if_changed();

    let colors = |i: usize| (button + COLOR_OFFSETS[i]) as *mut u32;
    let current = [*colors(0), *colors(1), *colors(2)];
    let good_color = if sell != 0 { state.config.sell_color } else { state.config.buy_color };

    // Remember the original colors the first time, or when the game reset them.
    let original = *state.original_colors.entry(button).or_insert(current);
    let original = if current != [good_color; 3] && current != original {
        state.original_colors.insert(button, current);
        current
    } else {
        original
    };

    let good = if ware < 20 {
        let w = ware as usize;
        if sell != 0 {
            let price = *((window + SELL_PRICES_OFFSET + ware * 4) as *const i32);
            let min = state.config.sell_min[w];
            min > 0 && price >= min
        } else {
            let price = *((window + BUY_PRICES_OFFSET + ware * 4) as *const i32);
            let max = state.config.buy_max[w];
            max > 0 && price > 0 && price <= max
        }
    } else {
        false
    };

    let wanted = if good { [good_color; 3] } else { original };
    for (i, color) in wanted.into_iter().enumerate() {
        *colors(i) = color;
    }
}

extern "C" {
    static buy_set_text_stub: c_void;
    static sell_set_text_stub: c_void;
}

// Entry: ecx = button, [esp] = return address, [esp+4] = text. esi = trade window, ebx = ware.
// The code after the original call doesn't use eax, ecx or edx.
global_asm!("
.global {buy_set_text_stub}
{buy_set_text_stub}:
    push 0
    jmp 2f

.global {sell_set_text_stub}
{sell_set_text_stub}:
    push 1

2:
    # [esp] = sell flag, [esp+4] = return address, [esp+8] = text
    push ecx
    push dword ptr [esp+12]
    call [{set_text}]
    pop ecx
    pop eax
    push eax
    push ebx
    push esi
    push ecx
    call {color_price_button}
    add esp, 16
    ret 4
",
buy_set_text_stub = sym buy_set_text_stub,
sell_set_text_stub = sym sell_set_text_stub,
set_text = sym SET_TEXT,
color_price_button = sym color_price_button);
