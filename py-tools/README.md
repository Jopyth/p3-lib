# py-tools

Python tools for supporting more than one build of the game (for now the international
`Patrician3.exe` and the German `Patrizier 2.exe`, Patrizier II Gold 1.1) and for testing
mods. They need Python 3.10+ and the packages `pefile`, `capstone` and `Pillow`:

```
python -m venv venv
venv\Scripts\python -m pip install pefile capstone pillow
```

All addresses in the Rust code are addresses in the international exe. `p3_api::addr()`
translates them for the running build, using the tables in `p3-api/src/version/`.

| Tool | Purpose |
|---|---|
| `addrmap.py` | Finds every address used in `p3-api`, `p3-modloader` and the mods in another build and writes the Rust table. Run it again after adding an address; `cargo test -p p3-api --lib` fails otherwise. |
| `add_import.py` | Makes a mod loader exe for any build: the exe plus an import of `p3_modloader.dll`. |
| `debuglog.py` | Starts the game under a minimal debugger and prints the `OutputDebugString` log, crashes and message boxes. Can save a screenshot or a memory dump. |
| `patchcheck.py` | Starts both builds with their loader exes and checks that every mod applied the same patches in both. |

## Supporting the German version

How to play with mods and the rules for writing them are in the [main README](../README.md).

```
python addrmap.py --intl path\to\Patrician3.exe --target "path\to\Patrizier 2.exe" --rs ..\p3-api\src\version\german.rs --report addrmap_german.md
python add_import.py "path\to\Patrizier 2.exe" "path\to\Patrizier 2_modloader.exe"
```

Copy `p3_modloader.dll` and the mods next to the German exe as usual, and start
`Patrizier 2_modloader.exe /norescheck`. On current Windows the German game can't read
the graphics test results from the registry (UAC registry virtualization), and
`/norescheck` skips that check. This happens without mods too. `/nointro` skips the
intro.

## Checking a build

```
python debuglog.py --seconds 30 --after "mod(s) loaded" 10 "path\to\Patrizier 2_modloader.exe" /nointro /norescheck
python patchcheck.py --intl Patrician3.exe Patrician3_modloader.exe --german "Patrizier 2.exe" "Patrizier 2_modloader.exe"
```
