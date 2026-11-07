"""Check that the mods patch the German exe the same way as the international one.

Usage:
    patchcheck.py --intl Patrician3.exe Patrician3_modloader.exe --german "Patrizier 2.exe" "Patrizier 2_modloader.exe"
                  [--german-args "/nointro /norescheck"] [--keep DIR | --reuse DIR]

Starts both mod loader exes under debuglog.py, reads the memory at every address in the German
address table right after "mod(s) loaded" (before the game's own WinMain runs), and compares
what the mods changed. For each address it compares the bytes that differ from the exe file:
both builds must have changed the same offsets to the same values. Values that differ are
accepted when they are rel32 or absolute pointers to the same place: the same offset in the
same mod DLL, or international and German addresses that belong together in the table.
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import pefile

TOOLS = Path(__file__).resolve().parent
PYTHON = sys.executable
TABLE = TOOLS.parent / "p3-api" / "src" / "version" / "german.rs"
# name -> (original exe, loader exe, arguments), filled from the command line
BUILDS = {}
DEFAULT_LENGTH = 32
LENGTHS = {0x0067AB30: 96, 0x004E639C: 64}  # hitbox data, verdict jump table
LOOKAHEAD = 8  # read past the window, so pointers that start inside it can be resolved
IMAGE_BASE = 0x00400000
DATA_SECTION = (0x00692000, 0x006F1CDC)  # .data of the international exe, changed at runtime anyway
MOD_WRITTEN_DATA = {0x0069AE40, 0x006A081C}  # .data places that mods write
EXPECTED_DIFFERENT = {0x006A081C: "market-hall title, a German text in the German build"}


def load_table():
    pairs = re.findall(r"\((0x[0-9a-f]+), (0x[0-9a-f]+)\)", TABLE.read_text())
    return [(int(a, 16), int(b, 16)) for a, b in pairs]


def file_bytes(exe):
    pe = pefile.PE(str(exe), fast_load=True)
    return bytes(pe.get_memory_mapped_image())


def run(name, regions, out_dir):
    original, loader, args = BUILDS[name]
    regions_file = out_dir / f"{name}_regions.txt"
    regions_file.write_text("".join(f"{a:#x} {n}\n" for a, n in regions))
    dump = out_dir / f"{name}_dump.json"
    cmd = [PYTHON, "-I", str(TOOLS / "debuglog.py"), "--seconds", "60", "--after", "mod(s) loaded", "2",
           "--dump", str(regions_file), str(dump), str(loader), *args]
    log = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    (out_dir / f"{name}_log.txt").write_text(log, encoding="utf-8")
    if not dump.exists():
        raise SystemExit(f"{name}: no dump, see {out_dir / (name + '_log.txt')}")
    return load_dump(dump)


def load_dump(path):
    data = json.loads(path.read_text())
    return {int(a, 16): bytes.fromhex(v) if v else None for a, v in data["regions"].items()}, data["modules"]


def changes(memory, original, address, length):
    before = original[address - IMAGE_BASE:address - IMAGE_BASE + length]
    return {i: memory[i] for i in range(min(len(memory), len(before))) if memory[i] != before[i]}


def locate(value, modules, table, build):
    """Describe where a pointer value points: a mod DLL offset or a (canonical) exe address."""
    if IMAGE_BASE <= value < 0x00800000:
        if build == "german":
            inverse = {de: intl for intl, de in table}
            return ("exe", inverse.get(value, ("unmapped", value)))
        return ("exe", value)
    candidates = [(base, name) for name, base in modules.items() if base <= value]
    if candidates:
        base, name = max(candidates)
        return ("module", Path(name).name.lower(), value - base)
    return ("unknown", value)


def compare(intl_site, de_site, ci, cg, mem_i, mem_g, mod_i, mod_g, table):
    # Compare the patched memory itself: a rel32 may keep a byte of the old value by chance.
    differing = sorted(k for k in set(ci) | set(cg) if mem_i[k] != mem_g[k])
    if not differing:
        return None
    # Group differing bytes into 4-byte pointer candidates and resolve them.
    checked = set()
    for k in differing:
        if k in checked:
            continue
        for start in range(k - 3, k + 1):
            if start < 0 or start + 4 > min(len(mem_i), len(mem_g)):
                continue
            vi = int.from_bytes(mem_i[start:start + 4], "little")
            vg = int.from_bytes(mem_g[start:start + 4], "little")
            options = []
            if start >= 1 and mem_i[start - 1] in (0xE8, 0xE9) and mem_g[start - 1] in (0xE8, 0xE9):
                options.append(((intl_site + start + 4 + vi) & 0xFFFFFFFF, (de_site + start + 4 + vg) & 0xFFFFFFFF))
            options.append((vi, vg))
            if any(locate(a, mod_i, table, "international") == locate(b, mod_g, table, "german") for a, b in options):
                checked.update(range(start, start + 4))
                break
        else:
            return f"byte {k}: international {mem_i[k]:#04x}, german {mem_g[k]:#04x} (no matching pointer interpretation)"
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--intl", nargs=2, required=True, type=Path, metavar=("EXE", "LOADER_EXE"))
    ap.add_argument("--german", nargs=2, required=True, type=Path, metavar=("EXE", "LOADER_EXE"))
    ap.add_argument("--german-args", default="/nointro /norescheck", help="arguments for the German loader exe")
    ap.add_argument("--keep", type=Path, help="keep dumps and logs in this directory")
    ap.add_argument("--reuse", type=Path, help="compare the dumps of an earlier --keep run instead of starting the game")
    args = ap.parse_args()

    BUILDS["international"] = (*args.intl, [])
    BUILDS["german"] = (*args.german, args.german_args.split())
    table = load_table()
    out_dir = args.reuse or args.keep or Path(tempfile.mkdtemp(prefix="patchcheck_"))
    out_dir.mkdir(parents=True, exist_ok=True)

    regions_i = [(a, LENGTHS.get(a, DEFAULT_LENGTH)) for a, _ in table]
    regions_g = [(g, LENGTHS.get(a, DEFAULT_LENGTH)) for a, g in table]
    if args.reuse:
        mem_i, mod_i = load_dump(out_dir / "international_dump.json")
        mem_g, mod_g = load_dump(out_dir / "german_dump.json")
    else:
        mem_i, mod_i = run("international", [(a, n + LOOKAHEAD) for a, n in regions_i], out_dir)
        mem_g, mod_g = run("german", [(g, n + LOOKAHEAD) for g, n in regions_g], out_dir)
    file_i = file_bytes(BUILDS["international"][0])
    file_g = file_bytes(BUILDS["german"][0])

    patched = problems = skipped = 0
    for (a, g), (_, n) in zip(table, regions_i):
        if DATA_SECTION[0] <= a < DATA_SECTION[1] and a not in MOD_WRITTEN_DATA:
            skipped += 1
            continue
        if mem_i.get(a) is None or mem_g.get(g) is None:
            print(f"{a:#010x} -> {g:#010x}: could not read memory")
            problems += 1
            continue
        ci = changes(mem_i[a], file_i, a, n)
        cg = changes(mem_g[g], file_g, g, n)
        if not ci and not cg:
            continue
        patched += 1
        problem = compare(a, g, ci, cg, mem_i[a], mem_g[g], mod_i, mod_g, table)
        if problem and a in EXPECTED_DIFFERENT:
            problem, status = None, f"OK (differs by design: {EXPECTED_DIFFERENT[a]})"
        else:
            status = "OK" if problem is None else f"MISMATCH: {problem}"
        problems += problem is not None
        print(f"{a:#010x} -> {g:#010x}: {len(ci):3} bytes changed, {status}")
    print(f"\n{patched} patched places compared, {problems} problems, {skipped} runtime data places skipped. Dumps and logs: {out_dir}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
