# p3-lib

A framework for modding Patrician 3, a mod loader, and the mods built with it.

## Supported game builds

| Build | Exe | Identified by (MD5 of the raw `.text` section) |
|---|---|---|
| International Patrician 3 (e.g. GOG) | `Patrician3.exe` | `d402f527f2cc5522f870965f5b354eea` |
| German Patrizier II Gold, version 1.1 (CD) | `Patrizier 2.exe` | `da8c9223f781bff9ed3ffe9c8cf0f9fd` |

The mod loader and every mod detect the build when they start. On any other build, for
example another CD print or an exe patched with a hex editor, the loader logs "Unsupported
game build" and the game starts without mods.

## Playing with mods

1. Get `p3_modloader.dll` and the mod DLLs: from a release of this repository
   (`p3_modloader.dll` and `mods.zip`), or build them yourself (see below). The same DLLs
   work with both builds.
2. Make a mod loader exe next to the game's exe. It is a copy of the exe that also loads
   `p3_modloader.dll`. You need Python 3.10 or newer with `pefile`
   (`pip install pefile`):

   ```
   python py-tools/add_import.py "Patrizier 2.exe" "Patrizier 2_modloader.exe"
   python py-tools/add_import.py Patrician3.exe Patrician3_modloader.exe
   ```

   Don't share the generated exe: it contains the game.
3. Put `p3_modloader.dll` next to the exe and the mod DLLs into a folder `mods` next to it.
4. Start the loader exe instead of the game.
   - German version: start it with `/norescheck`, e.g. through a shortcut to
     `"Patrizier 2_modloader.exe" /nointro /norescheck`. On current Windows the game
     can't read the graphics test results from the registry (UAC registry
     virtualization) and asks you to run `P2Setup.exe` again, even without mods.
     `/norescheck` skips that check, `/nointro` skips the intro. The Windows 7
     compatibility mode can help with other start problems.
5. To see what the mods do, watch the debug output with Sysinternals DebugView, or run
   `python py-tools/debuglog.py "Patrizier 2_modloader.exe" /nointro /norescheck`. A
   working start logs "Detected German game build" and "N mod(s) loaded successfully".
   `debug_navpoint_matrix.dll` is a tool for mod developers that draws navigation lines on
   the map; leave it out for normal play.

About `high_res`: it always switches the game to 1920×1080, but only in-game; the main
menu keeps its fixed resolution. The game's own images don't fully cover the bottom right
corner and the top bar at that size, in both builds.

## Building

Install Rust with the target `i686-pc-windows-msvc` (the workspace default, see
`.cargo/config.toml`) and the Visual Studio Build Tools with the C++ tools and the Windows
SDK. Then:

```
cargo build --release
```

The DLLs end up in `target/i686-pc-windows-msvc/release`.

## Developing for both builds

The two builds are compiled from the same code, but almost every address is shifted
(the German exe is about 6.8 KB smaller). `p3_api::version` detects the running build and
translates addresses with a table generated for each build
(`p3-api/src/version/german.rs`).

- Write every address as the address in the international `Patrician3.exe` and pass it
  through `p3_api::addr()`. Use `addr_ptr()` for typed pointers and `addr_rva()` for offsets
  that hooklet adds to the image base (`hook_call_rel32`, `hook_function_pointer`).
  Constants keep the international value and are translated where they are used; a `const`
  can't call `addr()`.
- Jump targets that `global_asm!` reads go into a `static mut X: u32 = 0x...;` that
  `start()` translates once (`X = p3_api::addr(X);`) before patching. Don't write absolute
  addresses into the assembler itself.
- In `start()`, right after `p3_api::mods::init_mod()`:

  ```rust
  if !p3_api::version::is_supported() {
      return p3_api::version::UNSUPPORTED_BUILD;
  }
  ```

  `addr()` panics on an unknown build, and a panic in `start()` aborts the game.
- Text drawn into the game's UI must be Latin-1: use `p3_api::text::latin1()` instead of
  `CString::new()`. A mod's own texts are `p3_api::text::Localized { en, de }`. Take ware
  names from `WareId::get_name()`, the game's ware order from
  `WareId::get_display_order()` and town names from `p3_api::town::get_town_name()`, so
  they match the game's language. German texts are often longer; check that they fit.
- After adding or changing an address, generate the table again and run the tests:

  ```
  python py-tools/addrmap.py --intl Patrician3.exe --target "Patrizier 2.exe" --rs p3-api/src/version/german.rs --report addrmap_german.md
  cargo test -p p3-api --lib
  ```

  The tests fail if an address used in `p3-api`, `p3-modloader` or a mod is missing from
  the table, or if an address literal is used without `addr()`. They don't need the game.
  `addrmap.py` needs both exes and `pefile` and `capstone`.
- To check a change in the game, use `py-tools/debuglog.py` (debug output, crashes, message
  boxes, screenshots) and `py-tools/patchcheck.py`, which compares the patches all mods
  applied in both builds.

To support another build, generate a table for it with `addrmap.py --name`, add its MD5 to
`KNOWN_BUILDS` and a variant to `GameVersion` in `p3-api/src/version/mod.rs`, and a case in
`try_addr()`. For a new language, also extend `Localized` in `p3-api/src/text.rs`. More
about the tools in [`py-tools/README.md`](py-tools/README.md).

## p3-aim

### Tests
For now, everything works only on x86:
`cargo test --target=i686-pc-windows-msvc -- --nocapture`
