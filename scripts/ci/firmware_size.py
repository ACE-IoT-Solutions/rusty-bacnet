#!/usr/bin/env python3
"""Check a firmware image's sizes against its budget (#1626).

  firmware_size.py <elf> <budget.toml> <target>

Prints the image's .text, .rodata, .data and .bss sizes next to the budget
for <target>, the largest objects in RAM, and any core::fmt functions linked
in. Exits 1 when a section is over budget, when more core::fmt functions are
linked in than the budget's `fmt_functions` allows, or when the budget lacks
an entry. Used by check-firmware-link.sh; see docs/ci.md, "Firmware link
test".

It reads the ELF's section headers and symbol table itself, so it needs only
Python 3.9 or later, no binutils. The budget file is the small subset of TOML
that `read_budget` accepts: comments, `[target]` tables and `key = integer`.
"""

import re
import struct
import sys
from pathlib import Path

SECTIONS = (".text", ".rodata", ".data", ".bss")
# What a budget suggests on failure: the measured size plus this much.
HEADROOM = 0.12
# Section types and flags, and symbol types, from the ELF specification.
SHT_SYMTAB = 2
SHF_ALLOC = 0x2
STT_OBJECT = 1
STT_FUNC = 2
# Objects in RAM to list, largest first.
TOP_RAM_OBJECTS = 4
# A Rust symbol in core::fmt or below, in either mangling scheme.
FMT_MARK = "4core3fmt"


class InputError(Exception):
    pass


def cstring(data, start):
    end = data.find(b"\0", start)
    if end < 0:
        raise InputError("unterminated string in the ELF")
    return data[start:end].decode(errors="replace")


def read_sections(data):
    """(name, type, flags, offset, size, link, entsize) for each section of
    a 32- or 64-bit little-endian ELF."""
    if data[:4] != b"\x7fELF" or data[5] != 1 or data[4] not in (1, 2):
        raise InputError("not a little-endian ELF")
    if data[4] == 1:
        shoff, shentsize, shnum, shstrndx = struct.unpack_from("<32xI10xHHH", data)
        layout = "<IIIIIIIIII"
    else:
        shoff, shentsize, shnum, shstrndx = struct.unpack_from("<40xQ10xHHH", data)
        layout = "<IIQQQQIIQQ"
    raw = [struct.unpack_from(layout, data, shoff + i * shentsize) for i in range(shnum)]
    names = raw[shstrndx][4]
    return [
        (cstring(data, names + name), kind, flags, offset, size, link, entsize)
        for name, kind, flags, _addr, offset, size, link, _info, _align, entsize in raw
    ]


def read_symbols(data, sections):
    """(name, size, type, section name) for each entry of .symtab."""
    symtab = next((s for s in sections if s[1] == SHT_SYMTAB), None)
    if symtab is None:
        return []
    _, _, _, offset, size, link, entsize = symtab
    strings = sections[link][3]
    wide = data[4] == 2
    symbols = []
    for at in range(offset, offset + size, entsize):
        if wide:
            name, info, _other, shndx, _value, length = struct.unpack_from("<IBBHQQ", data, at)
        else:
            name, _value, length, info, _other, shndx = struct.unpack_from("<IIIBBH", data, at)
        where = sections[shndx][0] if 0 < shndx < len(sections) else ""
        symbols.append((cstring(data, strings + name), length, info & 0xF, where))
    return symbols


def v0_identifier(name, i):
    """An identifier at name[i] in the v0 scheme: optional disambiguator,
    decimal length, optional `_` separator, the bytes."""
    if name.startswith("s", i):
        i = name.index("_", i) + 1
    digits = re.match(r"\d+", name[i:])
    if not digits:
        raise ValueError(name)
    i += digits.end()
    if name.startswith("_", i):
        i += 1
    length = int(digits.group())
    return name[i : i + length], i + length


def v0_path(name, i):
    """A path made of crate roots (C) and nested names (N); anything else,
    such as an impl or generic arguments, raises ValueError."""
    if name.startswith("C", i):
        return v0_identifier(name, i + 1)
    if name.startswith("N", i):
        parent, i = v0_path(name, i + 2)
        ident, i = v0_identifier(name, i)
        return (f"{parent}::{ident}" if ident else parent), i
    raise ValueError(name)


def demangle(name):
    """A readable path for a Rust symbol where that is easy, else the name."""
    if name.startswith("_R"):
        try:
            path, end = v0_path(name, 2)
            if end == len(name):
                return path
        except (ValueError, IndexError):
            pass
        return name
    legacy = re.fullmatch(r"_ZN(.*)E", name)
    if not legacy:
        return name
    rest, parts = legacy.group(1), []
    while rest:
        digits = re.match(r"\d+", rest)
        if not digits:
            return name
        start, length = digits.end(), int(digits.group())
        parts.append(rest[start : start + length])
        rest = rest[start + length :]
    if parts and re.fullmatch(r"h[0-9a-f]{16}", parts[-1]):
        parts.pop()
    return "::".join(parts)


def read_budget(path):
    """{table: {key: int}} from the TOML subset described in the module docs."""
    tables, table = {}, None
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        header = re.fullmatch(r"\[([A-Za-z0-9_.-]+)\]", line)
        entry = re.fullmatch(r"([A-Za-z0-9_]+)\s*=\s*(\d+)", line)
        if header:
            table = tables.setdefault(header.group(1), {})
        elif entry and table is not None:
            table[entry.group(1)] = int(entry.group(2))
        else:
            raise InputError(f"{path}:{number}: expected [target] or key = integer")
    return tables


def check(elf, budget_path, target):
    data = Path(elf).read_bytes()
    sections = read_sections(data)
    budget = read_budget(budget_path).get(target)
    if budget is None:
        raise InputError(f"{budget_path} has no [{target}] table")
    sizes = dict.fromkeys(SECTIONS, 0)
    for name, _kind, flags, _offset, size, _link, _entsize in sections:
        if flags & SHF_ALLOC and name in sizes:
            sizes[name] = size
    symbols = read_symbols(data, sections)
    fmt = sorted({demangle(s[0]) for s in symbols if s[2] == STT_FUNC and FMT_MARK in s[0]})

    rows = [(name, sizes[name], name.lstrip(".")) for name in SECTIONS]
    rows.append(("core::fmt", len(fmt), "fmt_functions"))
    failed = []
    print(f"  {'':<10}{'size':>8}{'budget':>8}")
    for label, measured, key in rows:
        limit = budget.get(key)
        if limit is None:
            raise InputError(f"{budget_path} has no {key} under [{target}]")
        over = measured > limit
        print(f"  {label:<10}{measured:>8}{limit:>8}{'  over budget' if over else ''}")
        if over:
            failed.append((key, measured))

    ram = [s for s in symbols if s[2] == STT_OBJECT and s[3] in (".data", ".bss")]
    print("  largest objects in RAM:")
    for name, size, _, where in sorted(ram, key=lambda s: -s[1])[:TOP_RAM_OBJECTS]:
        print(f"  {size:>10}  {where:<6} {demangle(name)}")
    for name in fmt:
        print(f"  core::fmt function: {name}")

    # The table first, then the errors under it, when both go to one log.
    sys.stdout.flush()
    for key, measured in failed:
        if key == "fmt_functions":
            advice = "find what formats, starting from the functions listed above"
        else:
            suggestion = -(-int(measured * (1 + HEADROOM)) // 16) * 16
            advice = (
                "if the growth is intended, raise the budget"
                f" ({suggestion} leaves {HEADROOM:.0%} headroom)"
            )
        print(
            f"error: {target}: {key} is {measured}, over its budget of"
            f" {budget[key]} in {budget_path}; {advice}. See docs/ci.md.",
            file=sys.stderr,
        )
    return 1 if failed else 0


def main(argv):
    if len(argv) != 4:
        print("usage: firmware_size.py <elf> <budget.toml> <target>", file=sys.stderr)
        return 2
    try:
        return check(*argv[1:])
    except (OSError, InputError, struct.error, IndexError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
