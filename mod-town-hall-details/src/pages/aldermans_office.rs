use std::ffi::CString;

use log::warn;
use p3_api::{
    data::{ddraw_set_constant_color, enums::WareId, ui_render_text_at},
    game_world::GAME_WORLD_PTR,
    missions::alderman_missions::{AldermanMissionDataPtr, FoundTownPtr},
    operations::OPERATIONS_PTR,
    scheduled_tasks::{scheduled_task::ScheduledTaskData, SCHEDULED_TASKS_PTR},
    text::{latin1, Localized},
    town::get_town_name,
    ui::{
        font::{self, get_normal_font},
        ui_town_hall_window::UITownHallWindowPtr,
    },
};

const COL_OFFSETS: &[i32; 2] = &[25, 400];
static TASK_RESCHEDULING_IN: Localized = Localized {
    en: "Rescheduling in",
    de: "Neuplanung in",
};
static TASK_RESCHEDULES_REMAINING: Localized = Localized {
    en: "Reschedule Counter",
    de: "Verbleibende Verschiebungen",
};
static TOWN: Localized = Localized { en: "Town", de: "Stadt" };
static EFFECTIVE_PRODUCTION: Localized = Localized {
    en: "Effective Production",
    de: "Wirksame Produktion",
};
static LOW_PRODUCTION: Localized = Localized {
    en: "Low Production",
    de: "Geringe Produktion",
};

pub(crate) unsafe fn draw_page(window: UITownHallWindowPtr) {
    let next_mission_index = window.get_next_mission_index();
    if next_mission_index == 0 {
        return;
    }

    if SCHEDULED_TASKS_PTR.get_merchant_alderman_mission_task_index(OPERATIONS_PTR.get_player_merchant_index()) != -1 {
        return;
    }

    let selected_mission_index = window.get_selected_alderman_mission_index();
    if selected_mission_index == 0xff {
        return;
    }
    let task_index = window.get_task_index(selected_mission_index);
    let task = SCHEDULED_TASKS_PTR.get_scheduled_task(task_index);
    let data = match task.get_data() {
        Some(e) => e,
        None => return,
    };

    let mission = match data {
        ScheduledTaskData::AldermanMission(mission) => mission,
        _ => return,
    };

    // Render general info
    ddraw_set_constant_color(0xff000000);
    font::ddraw_set_font(get_normal_font());
    let x = window.get_x();
    let mut y = window.get_y() + 60;

    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ui_render_text_at(x + COL_OFFSETS[0], y, TASK_RESCHEDULING_IN.get().to_bytes());
    font::ddraw_set_text_mode(font::TextMode::AlignRight);
    let rescheduling_in = task.get_due_timestamp() - GAME_WORLD_PTR.get_game_time_raw();
    let task_due_in_cstring = CString::new(format!("{rescheduling_in:#05x}")).unwrap();
    ui_render_text_at(x + COL_OFFSETS[1], y, task_due_in_cstring.to_bytes());
    y += 20;

    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ui_render_text_at(x + COL_OFFSETS[0], y, TASK_RESCHEDULES_REMAINING.get().to_bytes());
    font::ddraw_set_text_mode(font::TextMode::AlignRight);
    let reschedules_remaining = mission.get_reschedule_counter();
    let task_due_in_cstring = CString::new(format!("{reschedules_remaining}")).unwrap();
    ui_render_text_at(x + COL_OFFSETS[1], y, task_due_in_cstring.to_bytes());

    // Render mission-specific info
    match mission.get_data() {
        AldermanMissionDataPtr::FoundTownPtr(ptr) => render_aldermans_office_modifications_found_town(window, &ptr),
        AldermanMissionDataPtr::OverlandTradeRoute(_ptr) => {}
        AldermanMissionDataPtr::NotoriousPirate(_ptr) => {}
        AldermanMissionDataPtr::PirateHideout(_ptr) => {}
        AldermanMissionDataPtr::SupplyProblems(_ptr) => {}
    }
}

unsafe fn render_aldermans_office_modifications_found_town(window: UITownHallWindowPtr, data: &FoundTownPtr) {
    let town = data.get_town();
    let effective_raw = data.get_production_effective_raw();
    let effective = data.get_production_effective();
    let x = window.get_x();
    let mut y = window.get_y() + 100;

    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ui_render_text_at(x + COL_OFFSETS[0], y, TOWN.get().to_bytes());
    font::ddraw_set_text_mode(font::TextMode::AlignRight);
    let town_cstring = latin1(&get_town_name(town as u8).unwrap_or_else(|| format!("{town:?}")));
    ui_render_text_at(x + COL_OFFSETS[1], y, town_cstring.to_bytes());
    y += 20;

    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ui_render_text_at(x + COL_OFFSETS[0], y, EFFECTIVE_PRODUCTION.get().to_bytes());
    font::ddraw_set_text_mode(font::TextMode::AlignRight);
    let mut effective_string = String::new();
    for facility in effective {
        match facility {
            p3_api::data::enums::FacilityId::Militia => warn!("Unexpected facility {facility:?}"),
            p3_api::data::enums::FacilityId::Shipyard => warn!("Unexpected facility {facility:?}"),
            p3_api::data::enums::FacilityId::Construction => warn!("Unexpected facility {facility:?}"),
            p3_api::data::enums::FacilityId::Weaponsmith => warn!("Unexpected facility {facility:?}"),
            p3_api::data::enums::FacilityId::HuntingLodge => push_ware_names(&mut effective_string, &[WareId::Skins]),
            p3_api::data::enums::FacilityId::FishermansHouse => {
                if effective_raw & 0x20000 != 0 {
                    push_ware_names(&mut effective_string, &[WareId::WhaleOil])
                } else {
                    push_ware_names(&mut effective_string, &[WareId::Fish])
                }
            }
            p3_api::data::enums::FacilityId::Brewery => push_ware_names(&mut effective_string, &[WareId::Beer]),
            p3_api::data::enums::FacilityId::Workshop => push_ware_names(&mut effective_string, &[WareId::IronGoods]),
            p3_api::data::enums::FacilityId::Apiary => push_ware_names(&mut effective_string, &[WareId::Honey]),
            p3_api::data::enums::FacilityId::GrainFarm => push_ware_names(&mut effective_string, &[WareId::Grain]),
            p3_api::data::enums::FacilityId::CattleFarm => push_ware_names(&mut effective_string, &[WareId::Meat, WareId::Leather]),
            p3_api::data::enums::FacilityId::Sawmill => push_ware_names(&mut effective_string, &[WareId::Timber]),
            p3_api::data::enums::FacilityId::WeavingMill => push_ware_names(&mut effective_string, &[WareId::Cloth]),
            p3_api::data::enums::FacilityId::Saltery => push_ware_names(&mut effective_string, &[WareId::Salt]),
            p3_api::data::enums::FacilityId::Ironsmelter => push_ware_names(&mut effective_string, &[WareId::PigIron]),
            p3_api::data::enums::FacilityId::SheepFarm => push_ware_names(&mut effective_string, &[WareId::Wool]),
            p3_api::data::enums::FacilityId::Vineyard => push_ware_names(&mut effective_string, &[WareId::Wine]),
            p3_api::data::enums::FacilityId::Pottery => push_ware_names(&mut effective_string, &[WareId::Pottery]),
            p3_api::data::enums::FacilityId::Brickworks => push_ware_names(&mut effective_string, &[WareId::Bricks]),
            p3_api::data::enums::FacilityId::Pitchmaker => push_ware_names(&mut effective_string, &[WareId::Pitch]),
            p3_api::data::enums::FacilityId::HempFarm => push_ware_names(&mut effective_string, &[WareId::Hemp]),
        }
    }
    effective_string.pop();
    effective_string.pop();
    let effective_cstring = latin1(&effective_string);
    ui_render_text_at(x + COL_OFFSETS[1], y, effective_cstring.to_bytes());
    y += 20;

    font::ddraw_set_text_mode(font::TextMode::AlignLeft);
    ui_render_text_at(x + COL_OFFSETS[0], y, LOW_PRODUCTION.get().to_bytes());

    font::ddraw_set_text_mode(font::TextMode::AlignRight);
    let mut ineffective_string = String::new();
    if effective_raw & 0x20000 != 0 {
        ineffective_string.push_str(&WareId::Fish.get_name())
    }
}

/// Append the wares' names in the game's language, each followed by ", ".
fn push_ware_names(list: &mut String, wares: &[WareId]) {
    for ware in wares {
        list.push_str(&ware.get_name());
        list.push_str(", ");
    }
}
