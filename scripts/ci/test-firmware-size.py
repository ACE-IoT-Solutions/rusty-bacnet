#!/usr/bin/env python3
"""Regression tests for firmware_size.py (#1626), on small ELF files built
here: no binary fixtures, no dependencies. They check the parsing, totals
and budget logic, not real images; check-firmware-link.sh covers those."""

import contextlib
import importlib.util
import io
import struct
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "firmware_size", Path(__file__).with_name("firmware_size.py")
)
fs = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fs)

SHT_PROGBITS, SHT_SYMTAB, SHT_STRTAB, SHT_NOBITS = 1, 2, 3, 8
A, WA, AX = 0x2, 0x3, 0x6

# (name, type, flags, size) of each loaded section. The sizes are what the
# test asserts on; the sections hold no contents.
LOADED = [
    (".vector_table", SHT_PROGBITS, A, 1024),
    (".text", SHT_PROGBITS, AX, 3000),
    (".rodata", SHT_PROGBITS, A, 200),
    (".data", SHT_PROGBITS, WA, 16),
    (".bss", SHT_NOBITS, WA, 400),
    (".text.dummy", SHT_NOBITS, A, 8),
    (".stack", SHT_NOBITS, WA, 60000),
]
FUNC, OBJECT = 2, 1
# A v0-mangled function in .text, and one in core::fmt.
SCENARIO = "_RNvNtCs1_13firmware_link4mstp3run"
FMT_WRITE = "_RNvNtCs2_4core3fmt5write"
# 64-bit division: compiler-builtins' Rust implementation and the C-ABI
# routines in front of it, as a 32-bit ARM or RISC-V image links them.
U64_DIV_REM = "_RNvNtNtCs3_17compiler_builtins3int19specialized_div_rem11u64_div_rem"
DIV64_ROUTINES = ("__aeabi_uldivmod", "__udivdi3", "__umoddi3")
# 32-bit division, which a part without a divide instruction links too.
U32_DIV_REM = "_RNvNtNtCs3_17compiler_builtins3int19specialized_div_rem11u32_div_rem.llvm.1"
DIV32_ROUTINES = ("__aeabi_uidivmod", "__udivsi3")
# A static in .bss.
CORE = "_RNvNtCs1_13firmware_link5image4CORE"


def elf(
    symbols=((SCENARIO, 100, FUNC, ".text"), (CORE, 300, OBJECT, ".bss")),
    symtab=True,
    loaded=LOADED,
    wide=False,
):
    """A little-endian ELF, 64-bit if `wide`, with the `loaded` sections
    and, unless `symtab` is false, a symbol table holding `symbols`, given
    as (name, size, type, section name)."""
    names = [s[0] for s in loaded] + ([".symtab", ".strtab"] if symtab else []) + [".shstrtab"]
    shstrtab = b"\0"
    offsets = {}
    for name in names:
        offsets[name] = len(shstrtab)
        shstrtab += name.encode() + b"\0"
    index = {name: i + 1 for i, name in enumerate(names)}
    # Symbol and section-header layouts, and the ELF header's size, by class.
    if wide:
        symbol, section, header_size = "<IBBHQQ", "<IIQQQQIIQQ", 64
    else:
        symbol, section, header_size = "<IIIBBH", "<IIIIIIIIII", 52
    strtab, entries = b"\0", [bytes(struct.calcsize(symbol))]
    for name, size, kind, where in symbols:
        if wide:
            entries.append(struct.pack(symbol, len(strtab), kind, 0, index[where], 0, size))
        else:
            entries.append(struct.pack(symbol, len(strtab), 0, size, kind, 0, index[where]))
        strtab += name.encode() + b"\0"
    # (name, type, link, entsize, contents) of the sections with contents,
    # laid out after the ELF header, then the section headers.
    tables = [(".shstrtab", SHT_STRTAB, 0, 0, shstrtab)]
    if symtab:
        tables[:0] = [
            (".symtab", SHT_SYMTAB, index[".strtab"], struct.calcsize(symbol), b"".join(entries)),
            (".strtab", SHT_STRTAB, 0, 0, strtab),
        ]
    headers, contents, at = [bytes(struct.calcsize(section))], b"", header_size
    for name, kind, flags, size in loaded:
        headers.append(struct.pack(section, offsets[name], kind, flags, 0, 0, size, 0, 0, 4, 0))
    for name, kind, link, entsize, data in tables:
        headers.append(struct.pack(section, offsets[name], kind, 0, 0, at, len(data), link, 0, 1, entsize))
        contents += data
        at += len(data)
    ident = b"\x7fELF" + bytes([2 if wide else 1, 1, 1]) + bytes(9)
    # e_type, e_machine, e_version, e_entry, e_phoff, e_shoff, e_flags,
    # e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum, e_shstrndx.
    layout = "<HHIQQQIHHHHHH" if wide else "<HHIIIIIHHHHHH"
    header = ident + struct.pack(
        layout, 2, 40, 1, 0, 0, at, 0, header_size, 0, 0, struct.calcsize(section),
        len(headers), len(headers) - 1)
    return header + contents + b"".join(headers)


BUDGET = """\
# a comment
[target.image]
text = 3000
rodata = 200
data = 16
bss = 400
flash = 4240   # vector table, text, rodata and data
ram = 416
fmt_functions = 0
div64_functions = 0
"""


class FirmwareSizeTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)

    def tearDown(self):
        self.dir.cleanup()

    def write(self, name, content):
        path = self.root / name
        path.write_bytes(content) if isinstance(content, bytes) else path.write_text(content)
        return str(path)

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = fs.main(["firmware_size.py", *args])
        return status, out.getvalue(), err.getvalue()

    def test_sections_totals_and_nobits(self):
        loaded, sizes, _, listed = fs.measure(self.write("image", elf()))
        self.assertEqual(
            {key: sizes[key] for key in ("text", "rodata", "data", "bss")},
            {"text": 3000, "rodata": 200, "data": 16, "bss": 400},
        )
        # Flash: every loaded section with contents, .data's image included;
        # NOBITS sections take none.
        self.assertEqual(sizes["flash"], 1024 + 3000 + 200 + 16)
        # RAM: the writable sections, without the stack reservation; a
        # read-only NOBITS section counts toward neither.
        self.assertEqual(sizes["ram"], 16 + 400)
        totals = {name: totals for name, _, totals in loaded}
        self.assertEqual(totals[".data"], ("flash", "ram"))
        self.assertEqual(totals[".text.dummy"], ())
        self.assertEqual(totals[".stack"], ())
        self.assertEqual(listed, {"fmt_functions": [], "div64_functions": []})

    def test_heap_and_uninit_count_toward_ram(self):
        # riscv-rt's .heap is `_heap_size` octets reserved on purpose, and
        # cortex-m-rt's .uninit holds statics left uninitialized: both are
        # RAM the image takes. Only .stack, which fills what is left, isn't.
        loaded = LOADED + [(".heap", SHT_NOBITS, WA, 4096), (".uninit", SHT_NOBITS, WA, 64)]
        listing, sizes, _, _ = fs.measure(self.write("image", elf(loaded=loaded)))
        self.assertEqual(sizes["ram"], 16 + 400 + 4096 + 64)
        self.assertEqual(sizes["flash"], 1024 + 3000 + 200 + 16)
        totals = {name: totals for name, _, totals in listing}
        self.assertEqual(totals[".heap"], ("ram",))
        self.assertEqual(totals[".uninit"], ("ram",))

    def test_64_bit_elf(self):
        symbols = ((SCENARIO, 100, FUNC, ".text"), (FMT_WRITE, 40, FUNC, ".text"))
        _, sizes, read, listed = fs.measure(self.write("image", elf(symbols, wide=True)))
        self.assertEqual((sizes["text"], sizes["flash"], sizes["ram"]), (3000, 4240, 416))
        self.assertEqual([s[:2] for s in read[1:]], [(SCENARIO, 100), (FMT_WRITE, 40)])
        self.assertEqual(listed["fmt_functions"], ["core::fmt::write"])

    def test_within_budget_passes(self):
        status, out, err = self.run_main(
            self.write("image", elf()), self.write("budget.toml", BUDGET), "target.image")
        self.assertEqual(status, 0, err)
        self.assertIn("firmware_link::image::CORE", out)
        self.assertIn("firmware_link", out.split(".text by crate:")[1])
        self.assertEqual(err, "")

    def test_over_budget_fails_and_suggests(self):
        budget = BUDGET.replace("text = 3000", "text = 2999")
        status, out, err = self.run_main(
            self.write("image", elf()), self.write("budget.toml", budget), "target.image")
        self.assertEqual(status, 1)
        self.assertIn("over budget", out)
        # 3000 plus 12%, rounded up to 16 octets.
        self.assertIn("text is 3000, over its budget of 2999", err)
        self.assertIn("raise the budget (3360 leaves 12% headroom)", err)

    def test_loose_budget_gets_a_notice_not_a_failure(self):
        budget = BUDGET.replace("text = 3000", "text = 4000").replace("data = 16", "data = 64")
        status, _, err = self.run_main(
            self.write("image", elf()), self.write("budget.toml", budget), "target.image")
        self.assertEqual(status, 0)
        self.assertIn("notice: target.image: text is 3000", err)
        # A token allowance of 64 octets or less gets no notice.
        self.assertNotIn("data is", err)

    def test_count_under_its_limit_gets_a_notice_not_a_failure(self):
        # A count has no headroom, so any room under its limit gets a notice:
        # here one core::fmt function against a limit of 3, and no 64-bit
        # division against a limit of 1. A count at its limit gets none.
        symbols = ((SCENARIO, 100, FUNC, ".text"), (FMT_WRITE, 40, FUNC, ".text"))
        image = self.write("image", elf(symbols))
        budget = BUDGET.replace("fmt_functions = 0", "fmt_functions = 3").replace(
            "div64_functions = 0", "div64_functions = 1")
        status, out, err = self.run_main(image, self.write("a.toml", budget), "target.image")
        self.assertEqual(status, 0, err)
        self.assertIn("under its limit", out)
        self.assertIn(
            "notice: target.image: fmt_functions is 1, under its limit of 3;"
            " lower the limit to 1", err)
        self.assertIn("notice: target.image: div64_functions is 0, under its limit of 1", err)
        budget = BUDGET.replace("fmt_functions = 0", "fmt_functions = 1")
        status, _, err = self.run_main(image, self.write("b.toml", budget), "target.image")
        self.assertEqual((status, err), (0, ""))

    def test_fmt_function_fails(self):
        symbols = ((SCENARIO, 100, FUNC, ".text"), (FMT_WRITE, 40, FUNC, ".text"))
        status, out, err = self.run_main(
            self.write("image", elf(symbols)), self.write("budget.toml", BUDGET), "target.image")
        self.assertEqual(status, 1)
        self.assertIn("core::fmt function: core::fmt::write", out)
        self.assertIn("fmt_functions is 1, over its budget of 0", err)

    def test_64_bit_division_fails(self):
        symbols = [(SCENARIO, 100, FUNC, ".text"), (U64_DIV_REM, 866, FUNC, ".text")]
        symbols += [(name, 20, FUNC, ".text") for name in DIV64_ROUTINES]
        status, out, err = self.run_main(
            self.write("image", elf(symbols)), self.write("budget.toml", BUDGET), "target.image")
        self.assertEqual(status, 1)
        self.assertIn(
            "64-bit division function: compiler_builtins::int::specialized_div_rem::u64_div_rem",
            out,
        )
        for name in DIV64_ROUTINES:
            self.assertIn(f"64-bit division function: {name}", out)
        self.assertIn("div64_functions is 4, over its budget of 0", err)
        self.assertIn("divide in 32 bits", err)

    def test_32_bit_division_is_not_counted(self):
        symbols = [(SCENARIO, 100, FUNC, ".text"), (U32_DIV_REM, 390, FUNC, ".text")]
        symbols += [(name, 16, FUNC, ".text") for name in DIV32_ROUTINES]
        image = self.write("image", elf(symbols))
        _, sizes, _, listed = fs.measure(image)
        self.assertEqual((sizes["div64_functions"], listed["div64_functions"]), (0, []))
        status, _, err = self.run_main(image, self.write("budget.toml", BUDGET), "target.image")
        self.assertEqual(status, 0, err)

    def test_missing_symtab_fails_closed(self):
        status, _, err = self.run_main(
            self.write("image", elf(symtab=False)), self.write("budget.toml", BUDGET),
            "target.image")
        self.assertEqual(status, 1)
        self.assertIn("must not be stripped", err)

    def test_no_function_symbols_fails_closed(self):
        symbols = ((CORE, 300, OBJECT, ".bss"),)
        status, _, err = self.run_main(
            self.write("image", elf(symbols)), self.write("budget.toml", BUDGET), "target.image")
        self.assertEqual(status, 1)
        self.assertIn("must not be stripped", err)

    def test_missing_table_and_key_fail(self):
        image = self.write("image", elf())
        status, _, err = self.run_main(image, self.write("a.toml", BUDGET), "other.image")
        self.assertEqual((status, "no [other.image] table" in err), (1, True))
        budget = BUDGET.replace("ram = 416\n", "")
        status, _, err = self.run_main(image, self.write("b.toml", budget), "target.image")
        self.assertEqual((status, "has no ram" in err), (1, True))
        status, _, err = self.run_main(image, self.write("c.toml", "[t]\ntext = big\n"), "t")
        self.assertEqual((status, "expected [table] or key = integer" in err), (1, True))

    def test_suggest(self):
        status, out, _ = self.run_main("--suggest", self.write("image", elf()), "target.image")
        self.assertEqual(status, 0)
        self.assertEqual(
            out.splitlines(),
            ["[target.image]", "text = 3360", "rodata = 224", "data = 32", "bss = 448",
             "flash = 4752", "ram = 480", "fmt_functions = 0", "div64_functions = 0"],
        )

    def test_demangle_and_crate(self):
        self.assertEqual(fs.demangle(CORE), "firmware_link::image::CORE")
        self.assertEqual(fs.demangle("_ZN4core3fmt5write17h0123456789abcdefE"), "core::fmt::write")
        self.assertEqual(fs.crate_of(FMT_WRITE), "core")
        self.assertEqual(fs.crate_of("memcpy"), "(unmangled)")
        # A legacy-mangled trait impl is credited to the type's crate.
        impl = "_$LT$bacnet_bip_codec..table..Entries$LT$T$GT$$u20$as$u20$core..cmp..PartialEq$GT$"
        legacy = f"_ZN{len(impl)}{impl}2eq17h0123456789abcdefE"
        self.assertEqual(fs.crate_of(legacy), "bacnet_bip_codec")


if __name__ == "__main__":
    unittest.main()
