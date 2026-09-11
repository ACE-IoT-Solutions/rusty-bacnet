#!/usr/bin/env python3
"""Generate and compare deterministic public Python API manifests.

The tool reads ``.pyi`` files rather than importing an extension, so it can be
used for historical git refs and before a wheel is available.  A symbol is a
top-level class/function or a public method on a top-level class.
"""

from __future__ import annotations

import argparse
import ast
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


DEFAULT_STUB = "crates/rusty-bacnet/rusty_bacnet.pyi"


def read_source(spec: str) -> str:
    """Read a path, or ``GIT_REF:path`` when the path does not exist."""
    path = Path(spec)
    if path.exists():
        return path.read_text(encoding="utf-8")
    if ":" not in spec:
        raise FileNotFoundError(spec)
    return subprocess.check_output(
        ["git", "show", spec], text=True, encoding="utf-8"
    )


def rendered_signature(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    prefix = "async " if isinstance(node, ast.AsyncFunctionDef) else ""
    returns = f" -> {ast.unparse(node.returns)}" if node.returns else ""
    return f"{prefix}({ast.unparse(node.args)}){returns}"


def build_manifest(spec: str) -> dict[str, Any]:
    tree = ast.parse(read_source(spec), filename=spec, type_comments=True)
    exports: list[dict[str, Any]] = []
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
            bases = [ast.unparse(base) for base in node.bases]
            exports.append({"kind": "class", "name": node.name, "bases": bases})
            for member in node.body:
                if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    if member.name.startswith("_") and member.name != "__init__":
                        continue
                    exports.append(
                        {
                            "kind": "method",
                            "name": f"{node.name}.{member.name}",
                            "signature": rendered_signature(member),
                        }
                    )
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if not node.name.startswith("_"):
                exports.append(
                    {
                        "kind": "function",
                        "name": node.name,
                        "signature": rendered_signature(node),
                    }
                )
    exports.sort(key=lambda item: (item["name"], item["kind"]))
    return {"schema": 1, "source": spec, "exports": exports}


def symbol_map(manifest: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {f"{item['kind']}:{item['name']}": item for item in manifest["exports"]}


def run_manifest(args: argparse.Namespace) -> int:
    print(json.dumps(build_manifest(args.source), indent=2, sort_keys=True))
    return 0


def run_union(args: argparse.Namespace) -> int:
    manifests = {
        "old_local": build_manifest(args.old_local),
        "upstream": build_manifest(args.upstream),
        "integration": build_manifest(args.integration),
    }
    maps = {name: symbol_map(value) for name, value in manifests.items()}
    keys = sorted(set().union(*(mapping for mapping in maps.values())))
    rows = []
    upstream_losses = []
    for key in keys:
        present = {name: key in mapping for name, mapping in maps.items()}
        if present["integration"]:
            disposition = "present"
        elif present["upstream"]:
            disposition = "unexplained-upstream-loss"
            upstream_losses.append(key)
        else:
            disposition = "planned-local-port"
        rows.append(
            {
                "symbol": key,
                **present,
                "definitions": {
                    name: mapping.get(key) for name, mapping in maps.items()
                },
                "disposition": disposition,
            }
        )
    report = {
        "schema": 1,
        "sources": {name: value["source"] for name, value in manifests.items()},
        "counts": {name: len(value) for name, value in maps.items()},
        "summary": {
            "union": len(keys),
            "present": sum(row["disposition"] == "present" for row in rows),
            "planned_local_ports": sum(
                row["disposition"] == "planned-local-port" for row in rows
            ),
            "unexplained_upstream_losses": len(upstream_losses),
        },
        "rows": rows,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if args.fail_on_upstream_loss and upstream_losses else 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    manifest = commands.add_parser("manifest", help="emit one stub manifest")
    manifest.add_argument("source", nargs="?", default=DEFAULT_STUB)
    manifest.set_defaults(run=run_manifest)
    union = commands.add_parser("union", help="compare local, upstream, and integration")
    union.add_argument("--old-local", required=True)
    union.add_argument("--upstream", required=True)
    union.add_argument("--integration", default=DEFAULT_STUB)
    union.add_argument("--fail-on-upstream-loss", action="store_true")
    union.set_defaults(run=run_union)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        return args.run(args)
    except (OSError, SyntaxError, subprocess.CalledProcessError) as error:
        print(f"python-api-surface: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
