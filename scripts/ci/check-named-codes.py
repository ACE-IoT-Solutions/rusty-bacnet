#!/usr/bin/env python3
"""Check that the copies of the `named_codes!` macro stay byte-identical.

The macro builds the open wire-code newtypes of the no_std codecs:
`BvlcFunction` and friends in bacnet-bip-codec, `FrameType` in
bacnet-mstp-codec. Neither crate depends on the other, and the macro is
private to each, so each keeps a copy (#1684). This compares the two
`macro_rules! named_codes { ... }` definitions, from that line to its
closing brace, and prints a diff when they differ. The doc comments above
them may differ; the definitions may not.

    python3 scripts/ci/check-named-codes.py

Exit status: 0 when the copies match, 1 when they differ or one can't be
found.
"""

import difflib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COPIES = [
    "crates/bacnet-bip-codec/src/function.rs",
    "crates/bacnet-mstp-codec/src/frame.rs",
]
START = "macro_rules! named_codes {"


def macro_definition(path):
    """The macro's definition in `path`: its first line through the brace
    that closes it, or None when the file has no definition."""
    text = (ROOT / path).read_text()
    start = text.find(START)
    if start < 0 or text.find(START, start + 1) >= 0:
        return None
    depth = 0
    for index in range(start + len(START) - 1, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[start : index + 1]
    return None


def main():
    definitions = {}
    for path in COPIES:
        definition = macro_definition(path)
        if definition is None:
            print(f"error: no single `{START}` definition in {path}", file=sys.stderr)
            return 1
        definitions[path] = definition
    first, second = COPIES
    if definitions[first] == definitions[second]:
        print(f"OK: named_codes! is identical in {first} and {second}.")
        return 0
    diff = difflib.unified_diff(
        definitions[first].splitlines(keepends=True),
        definitions[second].splitlines(keepends=True),
        fromfile=first,
        tofile=second,
    )
    sys.stdout.writelines(diff)
    print(
        "\nerror: the named_codes! copies differ; change both together.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
