"""Make a mod loader exe: a copy of the game exe that also imports p3_modloader.dll.

Usage:
    add_import.py IN.exe OUT.exe [--dll p3_modloader.dll] [--function WinMain_hook] [--force]

Like the existing Patrician3_modloader.exe, the import table is copied into a new section
`.mod` and extended by one descriptor for the DLL. The descriptor imports one exported
function by name, because Windows skips DLLs whose import list is empty. The DLL's DllMain
then installs the hooks. Nothing else in the exe changes, so the .text section (and its MD5,
which p3-api uses to identify the build) stays the same.
"""

import argparse
import hashlib
import struct
import sys
from pathlib import Path

import pefile

SECTION_NAME = b".mod\0\0\0\0"
SECTION_CHARACTERISTICS = 0xC0000040  # initialized data, readable, writable
DESCRIPTOR_SIZE = 20
SECTION_HEADER_SIZE = 40


def align(value, alignment):
    return (value + alignment - 1) // alignment * alignment


def text_md5(data):
    pe = pefile.PE(data=data, fast_load=True)
    text = next(s for s in pe.sections if s.Name.rstrip(b"\0") == b".text")
    return hashlib.md5(data[text.PointerToRawData:text.PointerToRawData + text.SizeOfRawData]).hexdigest()


def imported_dlls(data):
    """DLL names from the raw import descriptors. pefile skips descriptors with an empty list."""
    pe = pefile.PE(data=data, fast_load=True)
    imports = pe.OPTIONAL_HEADER.DATA_DIRECTORY[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"]]
    names, rva = [], imports.VirtualAddress
    while True:
        _, _, _, name, first_thunk = struct.unpack("<IIIII", pe.get_data(rva, DESCRIPTOR_SIZE))
        if not name or not first_thunk:
            return names
        names.append(pe.get_string_at_rva(name).decode())
        rva += DESCRIPTOR_SIZE


def add_import(data, dll, function):
    pe = pefile.PE(data=data)
    out = bytearray(data)
    opt = pe.OPTIONAL_HEADER
    imports = opt.DATA_DIRECTORY[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"]]
    bound = opt.DATA_DIRECTORY[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_BOUND_IMPORT"]]

    names = [n.lower().removesuffix(".dll") for n in imported_dlls(data)]
    if dll.lower().removesuffix(".dll") in names:
        raise SystemExit(f"the exe already imports {dll}")
    if bound.VirtualAddress:
        raise SystemExit("the exe has bound imports, which this tool does not handle")

    # Room for one more section header.
    table = pe.sections[0].get_file_offset()
    new_header = table + len(pe.sections) * SECTION_HEADER_SIZE
    first_raw = min(s.PointerToRawData for s in pe.sections if s.PointerToRawData)
    if new_header + SECTION_HEADER_SIZE > min(opt.SizeOfHeaders, first_raw) or any(out[new_header:new_header + SECTION_HEADER_SIZE]):
        raise SystemExit("no room for another section header")

    # The existing descriptors, without the null terminator.
    old = pe.get_data(imports.VirtualAddress, len(pe.DIRECTORY_ENTRY_IMPORT) * DESCRIPTOR_SIZE)

    last = max(pe.sections, key=lambda s: s.VirtualAddress)
    va = align(last.VirtualAddress + max(last.Misc_VirtualSize, last.SizeOfRawData), opt.SectionAlignment)
    raw = align(len(out), opt.FileAlignment)

    # Layout: descriptors, new descriptor, terminator, OriginalFirstThunk list, FirstThunk list
    # (the IAT), hint/name entry of the function, DLL name.
    descriptors_size = len(old) + 2 * DESCRIPTOR_SIZE
    oft = va + descriptors_size
    ft = oft + 8
    hint_name = ft + 8
    hint_name_entry = struct.pack("<H", 0) + function.encode() + b"\0"
    hint_name_entry += bytes(len(hint_name_entry) % 2)
    name = hint_name + len(hint_name_entry)
    content = bytearray(old)
    content += struct.pack("<IIIII", oft, 0, 0, name, ft)
    content += bytes(DESCRIPTOR_SIZE)
    content += struct.pack("<II", hint_name, 0)
    content += struct.pack("<II", hint_name, 0)
    content += hint_name_entry
    content += dll.encode() + b"\0"

    virtual_size = align(len(content), opt.SectionAlignment)
    raw_size = align(len(content), opt.FileAlignment)

    out += bytes(raw - len(out))
    out += content + bytes(raw_size - len(content))
    struct.pack_into("<8sIIIIIIHHI", out, new_header, SECTION_NAME, virtual_size, va, raw_size, raw, 0, 0, 0, 0, SECTION_CHARACTERISTICS)

    struct.pack_into("<H", out, pe.FILE_HEADER.get_file_offset() + 2, pe.FILE_HEADER.NumberOfSections + 1)
    struct.pack_into("<I", out, opt.get_field_absolute_offset("SizeOfImage"), va + virtual_size)
    struct.pack_into("<II", out, imports.get_file_offset(), va, descriptors_size)
    return bytes(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("input", type=Path)
    ap.add_argument("output", type=Path)
    ap.add_argument("--dll", default="p3_modloader.dll")
    ap.add_argument("--function", default="WinMain_hook", help="a function the DLL exports, imported by name")
    ap.add_argument("--force", action="store_true", help="overwrite OUT.exe")
    args = ap.parse_args()

    if args.output.exists() and not args.force:
        raise SystemExit(f"{args.output} exists, pass --force to overwrite it")
    data = args.input.read_bytes()
    result = add_import(data, args.dll, args.function)

    imported = imported_dlls(result)
    assert imported == imported_dlls(data) + [args.dll], imported
    assert text_md5(result) == text_md5(data)
    entry = next(e for e in pefile.PE(data=result).DIRECTORY_ENTRY_IMPORT if e.dll.decode() == args.dll)
    assert [i.name.decode() for i in entry.imports] == [args.function], entry.imports

    args.output.write_bytes(result)
    print(f"wrote {args.output} (imports {', '.join(imported)}; .text md5 {text_md5(result)})", file=sys.stderr)


if __name__ == "__main__":
    main()
