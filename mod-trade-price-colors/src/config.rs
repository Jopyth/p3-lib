//! The price limits and colors, read from `mods/trade_price_colors.ini`.
//!
//! The file is written with the defaults if it doesn't exist, and read again whenever it
//! changes, so prices can be tuned while the game runs.

use std::{
    fmt::Write,
    fs,
    time::{Instant, SystemTime},
};

use log::{info, warn};
use p3_api::{
    data::enums::WareId,
    version::{game_version, GameVersion},
};

pub const CONFIG_PATH: &str = "./mods/trade_price_colors.ini";

/// The default green (ARGB), readable on the trade window's buttons.
const DEFAULT_COLOR: u32 = 0xFF00_8000;

/// Default limits from the Tippsammlung 2010 (patrizierforum.net), chapter 3.4.1 "Empfohlene
/// Preise aus der alten Tippsammlung": buy at most the upper end of the buy price, sell at least
/// the lower end of the sell price. The table has no buy price for fish and whale oil and no
/// row for hemp; those values (marked `true`) are estimates from Gesil's table (3.4.2) and the
/// arbitrage prices (9.1.2).
const DEFAULTS: [(WareId, i32, i32, bool); 20] = [
    (WareId::Beer, 40, 44, false),
    (WareId::PigIron, 900, 1200, false),
    (WareId::IronGoods, 320, 430, false),
    (WareId::Skins, 850, 900, false),
    (WareId::Fish, 400, 490, true),
    (WareId::Meat, 900, 1250, false),
    (WareId::Grain, 90, 140, false),
    (WareId::Spices, 400, 500, false),
    (WareId::Hemp, 400, 500, true),
    (WareId::Timber, 55, 75, false),
    (WareId::Honey, 110, 160, false),
    (WareId::Pottery, 170, 230, false),
    (WareId::Leather, 250, 300, false),
    (WareId::Pitch, 60, 100, false),
    (WareId::Salt, 25, 33, false),
    (WareId::WhaleOil, 75, 100, true),
    (WareId::Cloth, 220, 340, false),
    (WareId::Wine, 230, 350, false),
    (WareId::Wool, 900, 1300, false),
    (WareId::Bricks, 80, 130, false),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Colors,
    Buy,
    Sell,
}

pub struct Config {
    pub buy_color: u32,
    pub sell_color: u32,
    /// A buy price is good if it is greater than 0 and at most this (0 = never).
    pub buy_max: [i32; 20],
    /// A sell price is good if it is at least this (0 = never).
    pub sell_min: [i32; 20],
    modified: Option<SystemTime>,
    last_check: Option<Instant>,
}

fn german() -> bool {
    game_version() == GameVersion::German
}

impl Config {
    pub fn defaults() -> Self {
        let mut config = Self {
            buy_color: DEFAULT_COLOR,
            sell_color: DEFAULT_COLOR,
            buy_max: [0; 20],
            sell_min: [0; 20],
            modified: None,
            last_check: None,
        };
        for (ware, buy, sell, _) in DEFAULTS {
            config.buy_max[ware as usize] = buy;
            config.sell_min[ware as usize] = sell;
        }
        config
    }

    /// Read the file, or write it with the defaults if it doesn't exist.
    pub fn load() -> Self {
        let mut config = Self::defaults();
        match fs::metadata(CONFIG_PATH) {
            Ok(_) => config.read_file(),
            Err(_) => match fs::write(CONFIG_PATH, default_file_text()) {
                Ok(_) => info!("Wrote default settings to {CONFIG_PATH}"),
                Err(e) => warn!("Could not write {CONFIG_PATH}: {e}"),
            },
        }
        config.modified = fs::metadata(CONFIG_PATH).and_then(|m| m.modified()).ok();
        config
    }

    /// Read the file again if it changed. Checks at most once a second.
    pub fn reload_if_changed(&mut self) {
        if self.last_check.is_some_and(|t| t.elapsed().as_millis() < 1000) {
            return;
        }
        self.last_check = Some(Instant::now());
        let modified = fs::metadata(CONFIG_PATH).and_then(|m| m.modified()).ok();
        if modified.is_some() && modified != self.modified {
            let last_check = self.last_check;
            *self = Self::defaults();
            self.read_file();
            self.modified = modified;
            self.last_check = last_check;
        }
    }

    fn read_file(&mut self) {
        let bytes = match fs::read(CONFIG_PATH) {
            Ok(b) => b,
            Err(e) => {
                warn!("Could not read {CONFIG_PATH}: {e}");
                return;
            }
        };
        // Editors on Windows may save the file as ANSI (Latin-1) instead of UTF-8.
        let text = match String::from_utf8(bytes.clone()) {
            Ok(t) => t,
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        };
        self.parse(&text);
        info!("Read {CONFIG_PATH}");
    }

    fn parse(&mut self, text: &str) {
        let mut section = Section::None;
        for (n, line) in text.lines().enumerate() {
            let line = line.split([';', '#']).next().unwrap_or("").trim().trim_start_matches('\u{feff}');
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = match line[1..line.len() - 1].trim().to_lowercase().as_str() {
                    "farben" | "colors" | "colours" => Section::Colors,
                    "einkauf" | "kaufen" | "buy" => Section::Buy,
                    "verkauf" | "verkaufen" | "sell" => Section::Sell,
                    other => {
                        warn!("{CONFIG_PATH} line {}: unknown section [{other}]", n + 1);
                        Section::None
                    }
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warn!("{CONFIG_PATH} line {}: expected key = value", n + 1);
                continue;
            };
            let (key, value) = (key.trim().to_lowercase(), value.trim());
            match section {
                Section::Colors => {
                    let Some(color) = parse_color(value) else {
                        warn!("{CONFIG_PATH} line {}: bad color {value}", n + 1);
                        continue;
                    };
                    match key.as_str() {
                        "einkauf" | "kaufen" | "buy" => self.buy_color = color,
                        "verkauf" | "verkaufen" | "sell" => self.sell_color = color,
                        _ => warn!("{CONFIG_PATH} line {}: unknown color {key}", n + 1),
                    }
                }
                Section::Buy | Section::Sell => {
                    let Some(ware) = find_ware(&key) else {
                        warn!("{CONFIG_PATH} line {}: unknown ware {key}", n + 1);
                        continue;
                    };
                    let Ok(price) = value.replace(['.', '\''], "").parse::<i32>() else {
                        warn!("{CONFIG_PATH} line {}: bad price {value}", n + 1);
                        continue;
                    };
                    match section {
                        Section::Buy => self.buy_max[ware as usize] = price.max(0),
                        _ => self.sell_min[ware as usize] = price.max(0),
                    }
                }
                Section::None => warn!("{CONFIG_PATH} line {}: outside a section", n + 1),
            }
        }
    }
}

/// `RRGGBB` or `AARRGGBB`, optionally with `0x` or `#`.
fn parse_color(value: &str) -> Option<u32> {
    let hex = value.trim_start_matches("0x").trim_start_matches("0X").trim_start_matches('#');
    let color = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(0xFF00_0000 | color),
        8 => Some(color),
        _ => None,
    }
}

/// A ware by its name in the game's language or its English name (`IronGoods`, `iron goods`).
fn find_ware(key: &str) -> Option<WareId> {
    let squash = |s: &str| s.to_lowercase().replace([' ', '_', '-'], "");
    let key = squash(key);
    (0u8..20)
        .filter_map(num_traits::FromPrimitive::from_u8)
        .find(|ware: &WareId| squash(&ware.get_name()) == key || squash(&format!("{ware:?}")) == key)
}

fn default_file_text() -> String {
    let de = german();
    let mut t = String::new();
    if de {
        t.push_str(
            "; Preisfarben im Handelsfenster (Mod trade_price_colors)\n\
             ;\n\
             ; [Einkauf]: Der Preis unter \"Kaufen\" wird farbig, wenn er höchstens so hoch ist.\n\
             ; [Verkauf]: Der Preis unter \"Verkaufen\" wird farbig, wenn er mindestens so hoch ist.\n\
             ; 0 schaltet eine Ware ab. Änderungen gelten sofort, auch im laufenden Spiel.\n\
             ; Farben als RRGGBB oder AARRGGBB (hexadezimal).\n\
             ;\n\
             ; Startwerte: Tippsammlung 2010 (patrizierforum.net), Kapitel 3.4.1 \"Empfohlene Preise\n\
             ; aus der alten Tippsammlung\" (Einkauf: obere Grenze, Verkauf: untere Grenze).\n\
             ; Mit (Schätzung) markierte Werte fehlen dort und stammen aus 3.4.2 und 9.1.2.\n\n\
             [Farben]\nEinkauf = ",
        );
    } else {
        t.push_str(
            "; Price colors in the trade window (mod trade_price_colors)\n\
             ;\n\
             ; [buy]: the price under \"Buy\" is colored if it is at most this value.\n\
             ; [sell]: the price under \"Sell\" is colored if it is at least this value.\n\
             ; 0 switches a ware off. Changes apply immediately, also while the game runs.\n\
             ; Colors as RRGGBB or AARRGGBB (hexadecimal).\n\
             ;\n\
             ; Defaults: Tippsammlung 2010 (patrizierforum.net), chapter 3.4.1 (recommended prices\n\
             ; of the old tip collection; buy: upper end, sell: lower end). Values marked\n\
             ; (estimate) are missing there and come from chapters 3.4.2 and 9.1.2.\n\n\
             [colors]\nbuy = ",
        );
    }
    let _ = writeln!(t, "{:06X}", DEFAULT_COLOR & 0xFF_FFFF);
    let _ = writeln!(t, "{} = {:06X}\n", if de { "Verkauf" } else { "sell" }, DEFAULT_COLOR & 0xFF_FFFF);

    // The game sorts its ware list only after the mods are loaded, so sort the names here.
    let mut wares: Vec<WareId> = (0u8..20).filter_map(num_traits::FromPrimitive::from_u8).collect();
    wares.sort_by_key(|w| p3_api::text::latin1(&w.get_name()).into_bytes());
    let estimate = if de { "  ; (Schätzung)" } else { "  ; (estimate)" };
    for (header, buy) in [(if de { "[Einkauf]" } else { "[buy]" }, true), (if de { "[Verkauf]" } else { "[sell]" }, false)] {
        let _ = writeln!(t, "{header}");
        for &ware in &wares {
            let Some(&(_, b, s, est)) = DEFAULTS.iter().find(|d| d.0 == ware) else {
                continue;
            };
            let marked = est && (buy || ware == WareId::Hemp);
            let _ = writeln!(t, "{} = {}{}", ware.get_name(), if buy { b } else { s }, if marked { estimate } else { "" });
        }
        t.push('\n');
    }
    t
}
