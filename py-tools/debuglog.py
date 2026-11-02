"""Start a program under a minimal debugger and print its OutputDebugString messages.

Usage:
    debuglog.py [--seconds N] [--after PATTERN SECONDS] EXE [ARGS...]

Runs EXE in its own directory. Stops after N seconds (default 30), or SECONDS after a message
containing PATTERN, and then terminates the program. Crashes (second-chance exceptions) and
the exit code are reported. Replaces DebugView for quick automated checks of the mod loader.
"""

import argparse
import ctypes
import subprocess
import sys
import time
from ctypes import wintypes
from pathlib import Path

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

DEBUG_ONLY_THIS_PROCESS = 0x2
WAIT_MS = 200
EXCEPTION_DEBUG_EVENT = 1
CREATE_PROCESS_DEBUG_EVENT = 3
EXIT_PROCESS_DEBUG_EVENT = 5
LOAD_DLL_DEBUG_EVENT = 6
OUTPUT_DEBUG_STRING_EVENT = 8
DBG_CONTINUE = 0x00010002
DBG_EXCEPTION_NOT_HANDLED = 0x80010001
BREAKPOINTS = (0x80000003, 0x4000001F)  # native and WOW64 breakpoint


class STARTUPINFO(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("lpReserved", wintypes.LPWSTR), ("lpDesktop", wintypes.LPWSTR), ("lpTitle", wintypes.LPWSTR),
                ("dwX", wintypes.DWORD), ("dwY", wintypes.DWORD), ("dwXSize", wintypes.DWORD), ("dwYSize", wintypes.DWORD),
                ("dwXCountChars", wintypes.DWORD), ("dwYCountChars", wintypes.DWORD), ("dwFillAttribute", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("wShowWindow", wintypes.WORD), ("cbReserved2", wintypes.WORD),
                ("lpReserved2", ctypes.c_void_p), ("hStdInput", wintypes.HANDLE), ("hStdOutput", wintypes.HANDLE), ("hStdError", wintypes.HANDLE)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [("hProcess", wintypes.HANDLE), ("hThread", wintypes.HANDLE), ("dwProcessId", wintypes.DWORD), ("dwThreadId", wintypes.DWORD)]


def u32(buf, off):
    return int.from_bytes(buf[off:off + 4], "little")


user32 = ctypes.WinDLL("user32", use_last_error=True)
WNDENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)


def window_text(hwnd):
    buf = ctypes.create_unicode_buffer(1024)
    user32.GetWindowTextW(hwnd, buf, 1024)
    cls = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, cls, 256)
    return cls.value, buf.value


def screenshot(pid, path):
    """Save the largest visible top-level window of the process as an image."""
    from PIL import ImageGrab

    windows = []

    def top(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            rect = wintypes.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            windows.append((rect.right - rect.left) * (rect.bottom - rect.top) and (rect.left, rect.top, rect.right, rect.bottom))
        return True
    user32.EnumWindows(WNDENUMPROC(top), 0)
    windows = [w for w in windows if w]
    if not windows:
        print("--- no window for a screenshot", flush=True)
        return
    box = max(windows, key=lambda r: (r[2] - r[0]) * (r[3] - r[1]))
    ImageGrab.grab(bbox=box, all_screens=True).save(path)
    print(f"--- screenshot of {box} saved to {path}", flush=True)


def dump_regions(process, regions_path, out_path, modules):
    import json

    regions = {}
    for line in Path(regions_path).read_text().splitlines():
        if line.strip():
            address, length = line.split()
            address, length = int(address, 16), int(length)
            buf = ctypes.create_string_buffer(length)
            read = ctypes.c_size_t()
            ok = kernel32.ReadProcessMemory(process, ctypes.c_void_p(address), buf, length, ctypes.byref(read))
            regions[f"{address:#010x}"] = buf.raw[:read.value].hex() if ok else None
    Path(out_path).write_text(json.dumps({"regions": regions, "modules": modules}, indent=1))
    print(f"--- dumped {len(regions)} regions to {out_path}", flush=True)


def print_windows(pid):
    """Print the process's visible top-level windows, with the texts of dialog controls (message boxes)."""
    def top(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            cls, title = window_text(hwnd)
            print(f"window [{cls}] {title!r}", flush=True)

            def child(h, _):
                c, t = window_text(h)
                if t:
                    print(f"    [{c}] {t!r}", flush=True)
                return True
            user32.EnumChildWindows(hwnd, WNDENUMPROC(child), 0)
        return True
    user32.EnumWindows(WNDENUMPROC(top), 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=float, default=30)
    ap.add_argument("--after", nargs=2, metavar=("PATTERN", "SECONDS"))
    ap.add_argument("--verbose", action="store_true", help="also print debug events and loaded DLLs")
    ap.add_argument("--screenshot", type=Path, help="save the program's window as an image before terminating it")
    ap.add_argument("--dump", nargs=2, type=Path, metavar=("REGIONS", "OUT_JSON"),
                    help="when the --after pattern appears, read the memory regions listed in REGIONS (lines 'hex_address length') "
                         "and write them, plus the loaded modules, to OUT_JSON")
    ap.add_argument("exe", type=Path)
    ap.add_argument("args", nargs="*")
    args = ap.parse_args()

    user32.SetProcessDPIAware()
    exe = args.exe.resolve()
    si = STARTUPINFO(cb=ctypes.sizeof(STARTUPINFO))
    pi = PROCESS_INFORMATION()
    cmdline = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(exe), *args.args]))
    if not kernel32.CreateProcessW(None, cmdline, None, None, False, DEBUG_ONLY_THIS_PROCESS, None, str(exe.parent), ctypes.byref(si), ctypes.byref(pi)):
        raise SystemExit(f"CreateProcess failed: {ctypes.get_last_error()}")

    ptr_size = ctypes.sizeof(ctypes.c_void_p)
    union = 16 if ptr_size == 8 else 12
    event = ctypes.create_string_buffer(256)
    deadline = time.monotonic() + args.seconds
    exit_code = None
    modules = {}
    try:
        while time.monotonic() < deadline:
            if not kernel32.WaitForDebugEvent(event, WAIT_MS):
                continue
            raw = event.raw
            code, pid, tid = u32(raw, 0), u32(raw, 4), u32(raw, 8)
            status = DBG_CONTINUE
            if args.verbose:
                print(f"{time.strftime('%H:%M:%S')} event {code} {raw[union:union + 24].hex()}", flush=True)
            if code == LOAD_DLL_DEBUG_EVENT:
                handle = int.from_bytes(raw[union:union + ptr_size], "little")
                name = ctypes.create_unicode_buffer(520)
                if handle and kernel32.GetFinalPathNameByHandleW(wintypes.HANDLE(handle), name, 520, 0):
                    modules[name.value] = int.from_bytes(raw[union + ptr_size:union + 2 * ptr_size], "little")
                    if args.verbose:
                        print(f"{time.strftime('%H:%M:%S')} loaded {name.value}", flush=True)
                if handle:
                    kernel32.CloseHandle(wintypes.HANDLE(handle))
            if code == OUTPUT_DEBUG_STRING_EVENT:
                address = int.from_bytes(raw[union:union + ptr_size], "little")
                unicode = int.from_bytes(raw[union + ptr_size:union + ptr_size + 2], "little")
                length = int.from_bytes(raw[union + ptr_size + 2:union + ptr_size + 4], "little")
                size = length * (2 if unicode else 1)
                buf = ctypes.create_string_buffer(size)
                read = ctypes.c_size_t()
                if kernel32.ReadProcessMemory(pi.hProcess, ctypes.c_void_p(address), buf, size, ctypes.byref(read)):
                    text = buf.raw[:read.value].decode("utf-16-le" if unicode else "latin-1").rstrip("\0\r\n")
                    print(f"{time.strftime('%H:%M:%S')} {text}", flush=True)
                    if args.after and args.after[0] in text:
                        deadline = min(deadline, time.monotonic() + float(args.after[1]))
                        if args.dump:
                            dump_regions(pi.hProcess, args.dump[0], args.dump[1], modules)
            elif code == EXCEPTION_DEBUG_EVENT:
                exception = u32(raw, union)
                address = int.from_bytes(raw[union + 16:union + 16 + ptr_size], "little")
                first_chance = u32(raw, union + (152 if ptr_size == 8 else 80))
                if exception not in BREAKPOINTS:
                    status = DBG_EXCEPTION_NOT_HANDLED
                    if not first_chance:
                        print(f"!!! unhandled exception {exception:#010x} at {address:#010x} (thread {tid})", flush=True)
            elif code == EXIT_PROCESS_DEBUG_EVENT:
                exit_code = u32(raw, union)
                kernel32.ContinueDebugEvent(pid, tid, DBG_CONTINUE)
                break
            kernel32.ContinueDebugEvent(wintypes.DWORD(pid), wintypes.DWORD(tid), wintypes.DWORD(status))
    finally:
        if exit_code is None:
            print_windows(pi.dwProcessId)
            if args.screenshot:
                screenshot(pi.dwProcessId, args.screenshot)
            kernel32.TerminateProcess(pi.hProcess, 0)
            print(f"--- still running after the time limit, terminated", flush=True)
        else:
            print(f"--- process exited with code {exit_code:#x}", flush=True)
        kernel32.DebugActiveProcessStop(pi.dwProcessId)
        kernel32.CloseHandle(pi.hThread)
        kernel32.CloseHandle(pi.hProcess)


if __name__ == "__main__":
    main()
