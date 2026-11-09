"""Map the absolute addresses used by p3-lib from the international Patrician3.exe to another build.

Usage:
    addrmap.py --intl Patrician3.exe --target OTHER.exe [--src P3LIB] [--rs OUT.rs] [--report OUT.md] [--name NAME]

Every hex literal in the p3-lib sources that lies inside the international exe's image is
treated as an address and looked up in the target exe:

1. Functions are matched by a masked signature of their first bytes (absolute addresses and
   rel32 operands are wildcards). A signature has to be unique on both sides.
2. A code address is mapped through the nearest matched function before it, and the bytes
   from the function start up to the address (plus a tail) are compared under the same mask.
3. If that fails, the masked bytes around the address are searched near the expected place,
   then in the whole .text section.
4. Data addresses (and jump tables inside .text) are mapped through the instructions that
   reference them: each referencing instruction is mapped as code, and the operand is read
   from the target exe. All references have to agree.
5. Data addresses without a direct reference get the delta of their resolved neighbours, if
   the neighbours on both sides agree. These are marked as low confidence.
"""

import argparse
import bisect
import hashlib
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import pefile
from capstone import CS_ARCH_X86, CS_MODE_32, Cs

# The p3-lib workspace this script lives in.
DEFAULT_SRC = Path(__file__).resolve().parents[1]

# Crates whose addresses are mapped. p3-agent and the CLIs are not loaded into the game by the mod loader.
SCOPE = re.compile(r"^(p3-api|p3-modloader|mod-.*)$")

# Addresses that are not written as literals in the sources.
EXTRA = {}

SIG_LEN = 32
TAIL_LEN = 24
HEX_RE = re.compile(r"\b0x([0-9A-Fa-f]{6,8})\b")
# Offsets relative to the exe's image base (RVAs) that the sources pass to hooklet.
RVA_RES = [
    re.compile(r"\bhook_call_rel32\(\s*0x([0-9A-Fa-f]+)"),
    re.compile(r"\baddr_rva\(\s*0x([0-9A-Fa-f]+)"),
    re.compile(r"\bconst\s+\w*OFFSET\s*:\s*u32\s*=\s*0x([0-9A-Fa-f]+)\s*;"),
]

md = Cs(CS_ARCH_X86, CS_MODE_32)


class Image:
    def __init__(self, path):
        self.path = Path(path)
        data = self.path.read_bytes()
        self.pe = pefile.PE(data=data, fast_load=True)
        self.base = self.pe.OPTIONAL_HEADER.ImageBase
        self.mem = bytes(self.pe.get_memory_mapped_image())
        text = self.pe.sections[0]
        self.text_md5 = hashlib.md5(data[text.PointerToRawData:text.PointerToRawData + text.SizeOfRawData]).hexdigest()
        self.sections = []
        for s in self.pe.sections:
            lo = self.base + s.VirtualAddress
            self.sections.append((s.Name.rstrip(b"\0").decode(), lo, lo + max(s.Misc_VirtualSize, s.SizeOfRawData), lo + s.SizeOfRawData))
        _, self.text_lo, self.text_hi, _ = self.sections[0]
        self.end = max(hi for _, _, hi, _ in self.sections)

    def section(self, va):
        for name, lo, hi, raw_hi in self.sections:
            if lo <= va < hi:
                return name, va < raw_hi
        return None, False

    def bytes(self, va, n):
        rva = va - self.base
        return self.mem[rva:rva + n]

    def in_image(self, v):
        return self.base <= v < self.end

    def in_text(self, va):
        return self.text_lo <= va < self.text_hi


def masked(img, va, length):
    """Decode from va and return (bytes, mask) covering at least `length` bytes. mask[i] is True
    for bytes that may differ between builds (absolute addresses, rel32 operands)."""
    code = img.bytes(va, length + 16)
    out, mask = bytearray(), []
    for ins in md.disasm(code, va):
        bs = ins.bytes
        m = [False] * len(bs)
        for k in range(len(bs) - 3):
            if img.in_image(int.from_bytes(bs[k:k + 4], "little")):
                m[k:k + 4] = [True] * 4
        if bs[0] in (0xE8, 0xE9) and len(bs) == 5:
            m[1:5] = [True] * 4
        elif len(bs) == 6 and bs[0] == 0x0F and 0x80 <= bs[1] <= 0x8F:
            m[2:6] = [True] * 4
        out += bs
        mask += m
        if len(out) >= length:
            break
    return bytes(out), mask


def key(img, va, length=SIG_LEN):
    bs, mask = masked(img, va, length)
    if len(bs) < length:
        return None
    return bytes(0 if mask[i] else bs[i] for i in range(length)) + bytes(mask[:length])


def pattern(bs, mask):
    return re.compile(b"".join(b"." if m else re.escape(bytes([b])) for b, m in zip(bs, mask)), re.S)


def matches_at(img_a, va_a, img_b, va_b, length):
    """Compare img_a[va_a..] with img_b[va_b..] under the mask derived from img_a."""
    bs, mask = masked(img_a, va_a, length)
    if len(bs) < length:
        return False
    other = img_b.bytes(va_b, len(bs))
    return all(m or x == y for x, y, m in zip(bs, other, mask))


def call_targets(img):
    """Function starts: targets of E8 rel32 calls that are called at least twice."""
    text = img.bytes(img.text_lo, img.text_hi - img.text_lo)
    count = Counter()
    for m in re.finditer(b"\xE8", text):
        i = m.start()
        rel = int.from_bytes(text[i + 1:i + 5], "little", signed=True)
        t = img.text_lo + i + 5 + rel
        if img.text_lo <= t < img.text_hi:
            count[t] += 1
    return sorted(t for t, c in count.items() if c >= 2)


def match_functions(intl, target):
    by_key = defaultdict(list)
    for f in call_targets(target):
        k = key(target, f)
        if k:
            by_key[k].append(f)
    intl_keys = defaultdict(list)
    for f in call_targets(intl):
        k = key(intl, f)
        if k:
            intl_keys[k].append(f)
    fmap = {}
    for k, fs in intl_keys.items():
        if len(fs) == 1 and len(by_key.get(k, ())) == 1:
            fmap[fs[0]] = by_key[k][0]
    return fmap


class Mapper:
    def __init__(self, intl, target):
        self.intl, self.target = intl, target
        self.fmap = match_functions(intl, target)
        self.fstarts = sorted(self.fmap)
        self.target_text = target.bytes(target.text_lo, target.text_hi - target.text_lo)
        self.intl_text = intl.bytes(intl.text_lo, intl.text_hi - intl.text_lo)

    def nearest_function(self, va):
        i = bisect.bisect_right(self.fstarts, va) - 1
        return self.fstarts[i] if i >= 0 else None

    def map_code(self, va):
        """Return (target_va, method) or (None, reason)."""
        f = self.nearest_function(va)
        if f is not None and va - f < 0x4000:
            g = self.fmap[f]
            cand = g + (va - f)
            if matches_at(self.intl, f, self.target, g, va - f + TAIL_LEN) and matches_at(self.intl, va, self.target, cand, TAIL_LEN):
                return cand, f"function+{va - f:#x} (function {f:#x} -> {g:#x})"
        reason = "too little code to match"
        # Shorter signatures help when data (e.g. a switch jump table with absolute addresses)
        # follows the code shortly after the address.
        for length in (48, 32, 24, 16):
            bs, mask = masked(self.intl, va, length)
            if len(bs) < 16:
                break
            pat = pattern(bs, mask)
            if f is not None:
                exp = self.fmap[f] + (va - f)
                lo = max(exp - 0x800, self.target.text_lo)
                hits = [m.start() + lo for m in pat.finditer(self.target.bytes(lo, 0x1000))]
                if len(hits) == 1:
                    return hits[0], f"local signature of {length} bytes (expected {exp:#x}, off by {hits[0] - exp:+#x})"
                if exp in hits:
                    return exp, f"local signature at the expected place ({len(hits)} hits nearby)"
            hits = [m.start() + self.target.text_lo for m in pat.finditer(self.target_text)]
            if len(hits) == 1:
                return hits[0], f"global signature of {length} bytes"
            reason = f"signature has {len(hits)} hits"
        return None, reason

    def map_by_pointee(self, va):
        """Map a slot that holds a pointer to a string (e.g. a UI text table) through that string."""
        ptr = int.from_bytes(self.intl.bytes(va, 4), "little")
        if not self.intl.in_image(ptr):
            return None, "does not hold a pointer"
        text = self.intl.bytes(ptr, 256).split(b"\0")[0]
        if len(text) < 4 or not all(32 <= c < 127 for c in text):
            return None, "does not point to a string"
        slots = []
        for m in re.finditer(re.escape(b"\0" + text + b"\0"), self.target.mem):
            s = self.target.base + m.start() + 1
            slots += [self.target.base + r.start() for r in re.finditer(re.escape(s.to_bytes(4, "little")), self.target.mem)
                      if self.target.section(self.target.base + r.start())[0] != ".text"]
        if len(slots) != 1:
            return None, f"{len(slots)} pointers to {text!r}"
        return slots[0], f"pointer to {text.decode()!r}"

    def map_by_xref(self, va):
        """Map an address through the instructions that use it as an absolute operand."""
        needle = va.to_bytes(4, "little")
        votes = Counter()
        refs = 0
        for m in re.finditer(re.escape(needle), self.intl_text):
            off = self.intl.text_lo + m.start()
            for back in range(1, 8):
                start = off - back
                ins = next(md.disasm(self.intl.bytes(start, 16), start), None)
                if ins is None or ins.address + ins.size < off + 4:
                    continue
                if self.intl.bytes(start, ins.size).find(needle) != back:
                    continue
                mapped, _ = self.map_code(start)
                if mapped is not None:
                    votes[int.from_bytes(self.target.bytes(mapped + back, 4), "little")] += 1
                    refs += 1
                break
        if not votes:
            return None, "no mappable reference"
        (v, n), *rest = votes.most_common()
        if rest and not (n >= 3 and n >= 0.9 * refs):
            return None, "references disagree: " + ", ".join(f"{v:#x}x{c}" for v, c in votes.items())
        if rest:
            return v, f"xref ({n} of {refs} references agree)"
        return v, f"xref ({refs} references)"


def collect_sources(src, intl):
    found = defaultdict(list)
    for path in sorted(src.rglob("*.rs")):
        rel = path.relative_to(src)
        if not SCOPE.match(rel.parts[0]) or rel.parts[:3] == ("p3-api", "src", "version"):
            continue
        for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("//"):
                continue
            for m in HEX_RE.finditer(line):
                v = int(m.group(1), 16)
                if intl.in_image(v) and intl.section(v)[0]:
                    found[v].append(f"{rel.as_posix()}:{n}")
            for rva_re in RVA_RES:
                for m in rva_re.finditer(line):
                    v = intl.base + int(m.group(1), 16)
                    if intl.section(v)[0]:
                        found[v].append(f"{rel.as_posix()}:{n} (RVA {v - intl.base:#x})")
    return found


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--intl", required=True, type=Path, help="the international Patrician3.exe")
    ap.add_argument("--target", required=True, type=Path, help="the exe to map the addresses to")
    ap.add_argument("--src", default=DEFAULT_SRC, type=Path)
    ap.add_argument("--rs", type=Path, help="write the Rust table here")
    ap.add_argument("--report", type=Path, help="write a Markdown report here")
    ap.add_argument("--name", default="GERMAN", help="name of the Rust static")
    args = ap.parse_args()

    intl, target = Image(args.intl), Image(args.target)
    addrs = collect_sources(args.src, intl)
    for va, why in EXTRA.items():
        addrs[va].append(why)

    mapper = Mapper(intl, target)
    print(f"{len(mapper.fmap)} functions matched", file=sys.stderr)

    results = {}
    for va in sorted(addrs):
        sec, initialized = intl.section(va)
        if sec == ".text":
            res, how = mapper.map_code(va)
            if res is None:
                res, how2 = mapper.map_by_xref(va)
                how = how2 if res is not None else f"{how}; {how2}"
        else:
            res, how = mapper.map_by_xref(va)
            if res is None:
                res, how2 = mapper.map_by_pointee(va)
                how = how2 if res is not None else f"{how}; {how2}"
        results[va] = [res, how, "high" if res is not None else None]

    # Neighbour deltas for unresolved data addresses.
    for va, r in results.items():
        if r[0] is not None or intl.section(va)[0] == ".text":
            continue
        sec = intl.section(va)[0]
        resolved = sorted((a, x[0] - a) for a, x in results.items() if x[0] is not None and intl.section(a)[0] == sec)
        lower = [d for a, d in resolved if a < va][-1:]
        upper = [d for a, d in resolved if a > va][:1]
        if lower and upper and lower == upper:
            r[0], r[1], r[2] = va + lower[0], f"neighbour delta {lower[0]:+#x}; {r[1]}", "low"

    # Constant data must be equal in both builds.
    for va, r in results.items():
        sec, initialized = intl.section(va)
        if r[0] is not None and sec in (".rdata", ".data") and initialized and target.section(r[0])[1]:
            a, b = intl.bytes(va, 4), target.bytes(r[0], 4)
            if intl.in_image(int.from_bytes(a, "little")) and target.in_image(int.from_bytes(b, "little")):
                r[1] += f"; holds a pointer ({int.from_bytes(a, 'little'):#x} -> {int.from_bytes(b, 'little'):#x})"
            elif a != b:
                r[1] += f"; initial data differs ({intl.bytes(va, 4).hex()} vs {target.bytes(r[0], 4).hex()})"
                r[2] = "check"

    ok = sum(1 for r in results.values() if r[0] is not None)
    print(f"{ok}/{len(results)} addresses mapped", file=sys.stderr)

    if args.rs:
        lines = [
            "// Generated by py-tools/addrmap.py. Do not edit by hand.",
            f"// From: Patrician3.exe, .text md5 {intl.text_md5}",
            f"// To:   {target.path.name}, .text md5 {target.text_md5}",
            "// Sorted by the international address.",
            f"pub(crate) static {args.name}: &[(u32, u32)] = &[",
        ]
        for va, (res, how, conf) in sorted(results.items()):
            if res is None:
                lines.append(f"    // ({va:#010x}, ?), unresolved: {how}")
            else:
                note = "" if conf == "high" else f" // {conf}: {how}"
                lines.append(f"    ({va:#010x}, {res:#010x}),{note}")
        lines.append("];")
        args.rs.parent.mkdir(parents=True, exist_ok=True)
        args.rs.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")

    rows = []
    for va, (res, how, conf) in sorted(results.items()):
        sec = intl.section(va)[0]
        tgt = f"`{res:#010x}`" if res is not None else "**?**"
        delta = f"{res - va:+#x}" if res is not None else ""
        rows.append(f"| `{va:#010x}` | {sec} | {tgt} | {delta} | {conf or 'unresolved'} | {how} | {'<br>'.join(addrs[va])} |")
    report = "\n".join([
        f"# Address map: Patrician3.exe -> {target.path.name}",
        "",
        f"- International `.text` md5: `{intl.text_md5}`",
        f"- Target `.text` md5: `{target.text_md5}`",
        f"- Matched functions: {len(mapper.fmap)}",
        f"- Mapped: {ok} of {len(results)}",
        "",
        "| International | Section | Target | Delta | Confidence | Method | Used in |",
        "|---|---|---|---|---|---|---|",
        *rows,
        "",
    ])
    if args.report:
        args.report.write_text(report, encoding="utf-8", newline="\n")
    else:
        print(report)


if __name__ == "__main__":
    main()
