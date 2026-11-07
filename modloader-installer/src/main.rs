//! Makes a mod loader exe: a copy of the game's exe that also loads `p3_modloader.dll`.
//!
//! Started without arguments (e.g. by a double click in the game folder) it looks for
//! `Patrizier 2.exe` or `Patrician3.exe` next to itself and creates `<name>_modloader.exe`.
//!
//! Like the original `Patrician3_modloader.exe`, the import table is copied into a new section
//! `.mod` and extended by one descriptor for the DLL. The descriptor imports one exported
//! function by name, because Windows skips DLLs whose import list is empty. The DLL's
//! `DllMain` then installs the hooks. Nothing else in the exe changes, so the `.text` section,
//! by which p3-api identifies the build, stays the same. Same output as `py-tools/add_import.py`.

use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use p3_api::version::{exe_text_md5, version_of_exe, GameVersion};

const GAME_EXES: &[&str] = &["Patrizier 2.exe", "Patrician3.exe"];
const SECTION_NAME: &[u8; 8] = b".mod\0\0\0\0";
const SECTION_CHARACTERISTICS: u32 = 0xC000_0040; // initialized data, readable, writable
const DESCRIPTOR_SIZE: usize = 20;
const SECTION_HEADER_SIZE: usize = 40;
const BOUND_IMPORT_DIRECTORY: usize = 11;

#[derive(Parser)]
#[command(about = "Makes a mod loader exe for Patrician 3 / Patrizier II")]
struct Args {
    /// The game's exe. Default: Patrizier 2.exe or Patrician3.exe next to this program or in
    /// the current folder.
    game_exe: Option<PathBuf>,
    /// The loader exe to create. Default: <game exe name>_modloader.exe next to the game's exe.
    #[arg(long)]
    output: Option<PathBuf>,
    /// The DLL to load.
    #[arg(long, default_value = "p3_modloader.dll")]
    dll: String,
    /// A function the DLL exports, imported by name.
    #[arg(long, default_value = "WinMain_hook")]
    function: String,
    /// Overwrite an existing loader exe.
    #[arg(long)]
    force: bool,
    /// Also make a loader exe for an exe the mods don't support.
    #[arg(long)]
    any_build: bool,
}

fn main() -> ExitCode {
    // Started by a double click: keep the window open at the end.
    let interactive = std::env::args().len() == 1;
    let result = run(Args::parse());
    if let Err(e) = &result {
        eprintln!("Error: {e}");
    }
    if interactive {
        print!("\nPress Enter to close.");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().lines().next();
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(args: Args) -> Result<(), String> {
    let game_exe = match args.game_exe {
        Some(path) => path,
        None => find_game_exe()?,
    };
    let output = args.output.unwrap_or_else(|| loader_path(&game_exe));
    let input = fs::read(&game_exe).map_err(|e| format!("reading {} failed: {e}", game_exe.display()))?;

    let version = version_of_exe(&input);
    let md5 = exe_text_md5(&input).unwrap_or_else(|| "?".into());
    match version {
        GameVersion::International => println!("{}: international Patrician 3", game_exe.display()),
        GameVersion::German => println!("{}: German Patrizier II Gold 1.1", game_exe.display()),
        GameVersion::Unknown if args.any_build => {
            println!("{}: unknown build (.text md5 {md5}), the mods won't load", game_exe.display())
        }
        GameVersion::Unknown => {
            return Err(format!(
                "{} is not a build the mods support (.text md5 {md5}). The mods need the original exe of the international \
                 Patrician 3 or of the German Patrizier II Gold 1.1. Use --any-build to make a loader exe anyway.",
                game_exe.display()
            ))
        }
    }

    if output.exists() && !args.force {
        println!("{} already exists, nothing changed (--force overwrites it).", output.display());
    } else {
        create_loader(&input, &output, &args.dll, &args.function)?;
    }

    let folder = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if !folder.join(&args.dll).exists() {
        println!("Next: put {} next to it.", args.dll);
    }
    if !folder.join("mods").is_dir() {
        println!("Next: put the mod DLLs into a folder \"mods\" next to it.");
    }
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    if version == GameVersion::German {
        println!("Start \"{name}\" /norescheck instead of the game (/nointro skips the intro).");
        println!("Without /norescheck current Windows versions ask for P2Setup.exe, even without mods.");
    } else {
        println!("Start \"{name}\" instead of the game.");
    }
    Ok(())
}

fn create_loader(input: &[u8], output: &Path, dll: &str, function: &str) -> Result<(), String> {
    let result = add_import(input, dll, function)?;

    // The result must import the same DLLs plus ours, and keep the .text section.
    let before = Pe::parse(input)?.imported_dlls()?;
    let after = Pe::parse(&result)?.imported_dlls()?;
    if after.len() != before.len() + 1 || after[..before.len()] != before[..] || after[before.len()] != dll {
        return Err(format!("internal error: unexpected imports {after:?}"));
    }
    if exe_text_md5(&result) != exe_text_md5(input) {
        return Err("internal error: the .text section changed".into());
    }

    fs::write(output, &result).map_err(|e| format!("writing {} failed: {e}", output.display()))?;
    println!("Created {}", output.display());
    Ok(())
}

fn find_game_exe() -> Result<PathBuf, String> {
    let mut folders = Vec::new();
    if let Some(folder) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        folders.push(folder);
    }
    folders.push(PathBuf::from("."));
    folders
        .iter()
        .flat_map(|folder| GAME_EXES.iter().map(move |name| folder.join(name)))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "found no {} here. Put this program into the game folder, or pass the exe.",
                GAME_EXES.join(" or ")
            )
        })
}

fn loader_path(game_exe: &Path) -> PathBuf {
    let stem = game_exe.file_stem().unwrap_or_default().to_string_lossy();
    game_exe.with_file_name(format!("{stem}_modloader.exe"))
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

struct Section {
    header: usize,
    virtual_size: usize,
    virtual_address: usize,
    raw_size: usize,
    raw_offset: usize,
}

/// Just enough of a 32-bit PE file to extend its import table.
struct Pe<'a> {
    data: &'a [u8],
    nt_header: usize,
    optional_header: usize,
    sections: Vec<Section>,
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        let mut pe = Pe {
            data,
            nt_header: 0,
            optional_header: 0,
            sections: Vec::new(),
        };
        pe.nt_header = pe.u32(0x3C)? as usize;
        if data.get(pe.nt_header..pe.nt_header + 4) != Some(b"PE\0\0") {
            return Err("not a Windows exe".into());
        }
        pe.optional_header = pe.nt_header + 24;
        if pe.u16(pe.optional_header)? != 0x10B {
            return Err("not a 32-bit exe".into());
        }
        let section_count = pe.u16(pe.nt_header + 6)? as usize;
        let table = pe.optional_header + pe.u16(pe.nt_header + 20)? as usize;
        for i in 0..section_count {
            let header = table + i * SECTION_HEADER_SIZE;
            pe.sections.push(Section {
                header,
                virtual_size: pe.u32(header + 8)? as usize,
                virtual_address: pe.u32(header + 12)? as usize,
                raw_size: pe.u32(header + 16)? as usize,
                raw_offset: pe.u32(header + 20)? as usize,
            });
        }
        if pe.sections.is_empty() {
            return Err("the exe has no sections".into());
        }
        Ok(pe)
    }

    fn u16(&self, offset: usize) -> Result<u16, String> {
        let bytes = self.data.get(offset..offset + 2).ok_or("the exe is truncated")?;
        Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn u32(&self, offset: usize) -> Result<u32, String> {
        let bytes = self.data.get(offset..offset + 4).ok_or("the exe is truncated")?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    /// Offset of an entry of the data directory (VirtualAddress, then Size).
    fn data_directory(&self, index: usize) -> usize {
        self.optional_header + 96 + index * 8
    }

    fn rva_to_offset(&self, rva: usize) -> Result<usize, String> {
        self.sections
            .iter()
            .find(|s| (s.virtual_address..s.virtual_address + s.raw_size.max(s.virtual_size)).contains(&rva))
            .map(|s| rva - s.virtual_address + s.raw_offset)
            .ok_or_else(|| format!("address {rva:#x} is outside the sections"))
    }

    fn c_string(&self, rva: usize) -> Result<String, String> {
        let start = self.rva_to_offset(rva)?;
        let len = self.data[start..].iter().position(|&b| b == 0).ok_or("unterminated string")?;
        Ok(String::from_utf8_lossy(&self.data[start..start + len]).into_owned())
    }

    /// The DLL names in the import table, read from the raw descriptors.
    fn imported_dlls(&self) -> Result<Vec<String>, String> {
        let mut offset = self.rva_to_offset(self.u32(self.data_directory(1))? as usize)?;
        let mut names = Vec::new();
        loop {
            let name = self.u32(offset + 12)? as usize;
            let first_thunk = self.u32(offset + 16)?;
            if name == 0 || first_thunk == 0 {
                return Ok(names);
            }
            names.push(self.c_string(name)?);
            offset += DESCRIPTOR_SIZE;
        }
    }
}

fn add_import(input: &[u8], dll: &str, function: &str) -> Result<Vec<u8>, String> {
    let pe = Pe::parse(input)?;
    let base_name = |name: &str| name.to_ascii_lowercase().trim_end_matches(".dll").to_string();
    let existing = pe.imported_dlls()?;
    if existing.iter().any(|name| base_name(name) == base_name(dll)) {
        return Err(format!("the exe already imports {dll}, it is a loader exe already"));
    }
    if pe.u32(pe.data_directory(BOUND_IMPORT_DIRECTORY))? != 0 {
        return Err("the exe has bound imports, which this program doesn't handle".into());
    }

    // Room for one more section header.
    let new_header = pe.sections[0].header + pe.sections.len() * SECTION_HEADER_SIZE;
    let size_of_headers = pe.u32(pe.optional_header + 60)? as usize;
    let first_raw = pe.sections.iter().map(|s| s.raw_offset).filter(|&o| o != 0).min().unwrap_or(size_of_headers);
    if new_header + SECTION_HEADER_SIZE > size_of_headers.min(first_raw) || input[new_header..new_header + SECTION_HEADER_SIZE].iter().any(|&b| b != 0) {
        return Err("no room for another section header".into());
    }

    // The existing descriptors, without the null terminator.
    let import_offset = pe.rva_to_offset(pe.u32(pe.data_directory(1))? as usize)?;
    let old = &input[import_offset..import_offset + existing.len() * DESCRIPTOR_SIZE];

    let section_alignment = pe.u32(pe.optional_header + 32)? as usize;
    let file_alignment = pe.u32(pe.optional_header + 36)? as usize;
    let last = pe.sections.iter().max_by_key(|s| s.virtual_address).unwrap();
    let va = align(last.virtual_address + last.virtual_size.max(last.raw_size), section_alignment);
    let raw = align(input.len(), file_alignment);

    // Layout: descriptors, new descriptor, terminator, OriginalFirstThunk list, FirstThunk list
    // (the IAT), hint/name entry of the function, DLL name.
    let descriptors_size = old.len() + 2 * DESCRIPTOR_SIZE;
    let original_first_thunk = va + descriptors_size;
    let first_thunk = original_first_thunk + 8;
    let hint_name = first_thunk + 8;
    let mut hint_name_entry = vec![0u8, 0];
    hint_name_entry.extend_from_slice(function.as_bytes());
    hint_name_entry.push(0);
    if hint_name_entry.len() % 2 == 1 {
        hint_name_entry.push(0);
    }
    let name = hint_name + hint_name_entry.len();

    let dword = |v: usize| (v as u32).to_le_bytes();
    let mut content = old.to_vec();
    for v in [original_first_thunk, 0, 0, name, first_thunk] {
        content.extend_from_slice(&dword(v));
    }
    content.extend_from_slice(&[0; DESCRIPTOR_SIZE]);
    for _ in 0..2 {
        content.extend_from_slice(&dword(hint_name));
        content.extend_from_slice(&dword(0));
    }
    content.extend_from_slice(&hint_name_entry);
    content.extend_from_slice(dll.as_bytes());
    content.push(0);

    let virtual_size = align(content.len(), section_alignment);
    let raw_size = align(content.len(), file_alignment);

    let mut out = input.to_vec();
    out.resize(raw, 0);
    out.extend_from_slice(&content);
    out.resize(raw + raw_size, 0);

    let mut header = Vec::with_capacity(SECTION_HEADER_SIZE);
    header.extend_from_slice(SECTION_NAME);
    for v in [virtual_size, va, raw_size, raw, 0, 0] {
        header.extend_from_slice(&dword(v));
    }
    header.extend_from_slice(&[0; 4]); // NumberOfRelocations, NumberOfLinenumbers
    header.extend_from_slice(&SECTION_CHARACTERISTICS.to_le_bytes());
    out[new_header..new_header + SECTION_HEADER_SIZE].copy_from_slice(&header);

    let count = pe.sections.len() as u16 + 1;
    out[pe.nt_header + 6..pe.nt_header + 8].copy_from_slice(&count.to_le_bytes());
    out[pe.optional_header + 56..pe.optional_header + 60].copy_from_slice(&dword(va + virtual_size));
    let import_directory = pe.data_directory(1);
    out[import_directory..import_directory + 4].copy_from_slice(&dword(va));
    out[import_directory + 4..import_directory + 8].copy_from_slice(&dword(descriptors_size));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_path_is_next_to_the_exe() {
        assert_eq!(loader_path(Path::new("a/Patrizier 2.exe")), Path::new("a/Patrizier 2_modloader.exe"));
    }

    #[test]
    fn rejects_files_that_are_not_exes() {
        assert!(add_import(b"not an exe at all, just some bytes", "p3_modloader.dll", "WinMain_hook").is_err());
    }

    /// Checks real game exes, which are not in the repository. Run with
    /// `P3_GAME_EXES="path/Patrician3.exe;path/Patrizier 2.exe" cargo test -p modloader-installer -- --ignored`.
    #[test]
    #[ignore]
    fn game_exes_get_the_import() {
        let exes = std::env::var("P3_GAME_EXES").expect("set P3_GAME_EXES to a ;-separated list of exes");
        for exe in exes.split(';').filter(|e| !e.is_empty()) {
            let input = fs::read(exe).unwrap();
            let output = add_import(&input, "p3_modloader.dll", "WinMain_hook").unwrap();
            let dlls = Pe::parse(&output).unwrap().imported_dlls().unwrap();
            assert_eq!(dlls.last().unwrap(), "p3_modloader.dll", "{exe}");
            assert_eq!(exe_text_md5(&output), exe_text_md5(&input), "{exe}");
            assert!(add_import(&output, "p3_modloader.dll", "WinMain_hook").is_err(), "{exe} imported twice");
        }
    }
}
