#!/usr/bin/env python3
"""Generate deterministic Phase 0 reconciliation manifests from pinned Git refs.

The inventories are intentionally lexical.  They require only Python 3.11+ and
Git, and read objects directly from Git so the checked-out integration branch
cannot contaminate either baseline.
"""

from __future__ import annotations

import ast
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent
LOCAL = "archive/dev-pre-upstream-reconciliation-20260911"
UPSTREAM = "archive/upstream-dev-pinned-20260911"
BASE = "6b9ac4f0f2dfadbf6dbbda88c46ad6a0b0aa43d2"


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE
    ).stdout


def paths(ref: str) -> list[str]:
    return sorted(git("ls-tree", "-r", "--name-only", ref).splitlines())


def blob(ref: str, path: str) -> str:
    return git("show", f"{ref}:{path}")


def write(path: Path, header: str, rows: set[str] | list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    body = "\n".join(sorted(set(rows)))
    path.write_text(f"{header}\n{body}\n", encoding="utf-8")


def workspace(ref: str, tree: list[str]) -> list[str]:
    root = tomllib.loads(blob(ref, "Cargo.toml"))
    rows = [f"workspace\tmember\t{x}" for x in root["workspace"].get("members", [])]
    rows += [f"workspace\tdefault-member\t{x}" for x in root["workspace"].get("default-members", [])]
    for manifest in (p for p in tree if p == "Cargo.toml" or p.endswith("/Cargo.toml")):
        try:
            data = tomllib.loads(blob(ref, manifest))
        except (subprocess.CalledProcessError, tomllib.TOMLDecodeError):
            continue
        package = data.get("package", {}).get("name", "<workspace>")
        for feature, members in data.get("features", {}).items():
            value = ",".join(str(x) for x in members)
            rows.append(f"{manifest}\tfeature\t{package}::{feature}=[{value}]")
    return rows


RUST_ITEM = re.compile(
    r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+|unsafe\s+|const\s+)*"
    r"(fn|struct|enum|trait|type|const|static|mod|use)\s+([^\s(<:{;=]+)"
)
IMPL = re.compile(r"^\s*impl(?:<[^>]*>)?\s+(?:[^\s]+\s+for\s+)?([^\s<{]+)")


def rust_public(ref: str, tree: list[str]) -> list[str]:
    rows: list[str] = []
    for path in (p for p in tree if p.endswith(".rs")):
        owner = ""
        depth = 0
        for lineno, line in enumerate(blob(ref, path).splitlines(), 1):
            match_impl = IMPL.match(line)
            if match_impl and "{" in line:
                owner = match_impl.group(1)
                depth = line.count("{") - line.count("}")
            elif owner:
                depth += line.count("{") - line.count("}")
                if depth <= 0:
                    owner = ""
            match = RUST_ITEM.match(line)
            if not match:
                continue
            kind, name = match.groups()
            if kind == "fn" and owner:
                name = f"{owner}::{name}"
                kind = "method"
            rows.append(f"{path}\t{lineno}\t{kind}\t{name}")
    return rows


def signature(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    try:
        return ast.unparse(node.args)
    except Exception:
        return "<unavailable>"


def python_api(ref: str, tree: list[str]) -> list[str]:
    rows: list[str] = []
    candidates = [p for p in tree if p.endswith((".pyi", ".py")) and "test" not in p.lower()]
    for path in candidates:
        try:
            module = ast.parse(blob(ref, path))
        except (SyntaxError, UnicodeDecodeError):
            continue
        for node in module.body:
            if isinstance(node, ast.ClassDef):
                bases = ",".join(ast.unparse(x) for x in node.bases)
                kind = "exception" if "Error" in node.name or "Exception" in bases else "class"
                rows.append(f"{path}\t{node.lineno}\t{kind}\t{node.name}\t({bases})")
                for child in node.body:
                    if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        rows.append(
                            f"{path}\t{child.lineno}\tmethod\t{node.name}.{child.name}\t({signature(child)})"
                        )
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                rows.append(f"{path}\t{node.lineno}\tfunction\t{node.name}\t({signature(node)})")
    exception_re = re.compile(r"create_exception!\([^,]+,\s*([A-Za-z_][A-Za-z0-9_]*)")
    for path in (p for p in tree if p.endswith(".rs")):
        for lineno, line in enumerate(blob(ref, path).splitlines(), 1):
            for name in exception_re.findall(line):
                rows.append(f"{path}\t{lineno}\texception\t{name}\t(PyO3 create_exception)")
    return rows


def tests(ref: str, tree: list[str]) -> tuple[list[str], list[str]]:
    files: list[str] = []
    names: list[str] = []
    for path in tree:
        is_test_file = (
            "/tests/" in f"/{path}"
            or path.startswith("tests/")
            or Path(path).name.startswith("test_")
            or Path(path).stem.endswith("_test")
        )
        if not is_test_file or not path.endswith((".rs", ".py")):
            continue
        files.append(path)
        source = blob(ref, path)
        if path.endswith(".py"):
            try:
                module = ast.parse(source)
            except SyntaxError:
                continue
            for node in ast.walk(module):
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test"):
                    names.append(f"{path}\t{node.lineno}\t{node.name}")
        else:
            pending = False
            for lineno, line in enumerate(source.splitlines(), 1):
                if re.search(r"#\[(?:tokio::)?test(?:\([^]]*\))?\]", line):
                    pending = True
                    continue
                if pending:
                    found = re.search(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", line)
                    if found:
                        names.append(f"{path}\t{lineno}\t{found.group(1)}")
                        pending = False
    return files, names


def conformance(ref: str, tree: list[str]) -> tuple[list[str], list[str]]:
    rows: list[str] = []
    claims: list[str] = []
    ledger = "docs/conformance/bacnet-135-2020.json"
    if ledger in tree:
        data = json.loads(blob(ref, ledger))
        for row in data.get("rows", []):
            rows.append(
                f"{row.get('id', '')}\t{row.get('standard_anchor', '')}\t{row.get('status', '')}"
            )
            for claim in row.get("public_claims", []):
                claims.append(f"{ledger}\trow:{row.get('id', '')}\t{claim}")
    claim_files = [p for p in tree if p == "README.md" or p == "docs/conformance/support-summary.md"]
    signal = re.compile(r"\b(support(?:ed|s|ing)?|implement(?:ed|s)?|conform(?:ance|ant)?|BIBB|PICS)\b", re.I)
    for path in claim_files:
        for lineno, line in enumerate(blob(ref, path).splitlines(), 1):
            clean = " ".join(line.strip().split())
            if clean and signal.search(clean):
                claims.append(f"{path}\tline:{lineno}\t{clean}")
    return rows, claims


def classify_local_only() -> list[str]:
    local = set(git("diff", "--name-only", f"{BASE}..{LOCAL}").splitlines())
    upstream = set(git("diff", "--name-only", f"{BASE}..{UPSTREAM}").splitlines())
    rows = []
    for path in sorted(local - upstream):
        lower = path.lower()
        if path == "Cargo.lock":
            disposition, reason = "replace-with-upstream", "lockfile must be regenerated from the upstream-first graph"
        elif lower.startswith("docs/plans/") or "evidence" in lower:
            disposition, reason = "evidence-only", "historical planning/evidence; retain without treating as product behavior"
        elif "/tests/" in f"/{lower}" or Path(path).name.startswith("test_"):
            disposition, reason = "port", "capability acceptance evidence must move with its behavior"
        elif "bacnet-runtime" in lower:
            disposition, reason = "port", "repository-owned aggregate runtime is absent upstream"
        elif any(x in lower for x in ("bbmd", "segmentation", "transaction", "cov")):
            disposition, reason = "needs-design", "port contract onto upstream ownership and safety invariants"
        elif lower.endswith((".md", ".json")):
            disposition, reason = "evidence-only", "review claim-by-claim after implementation"
        else:
            disposition, reason = "port", "unique local path; preserve behavior unless tranche review proves otherwise"
        rows.append(f"{path}\t{disposition}\t{reason}")
    return rows


def main() -> None:
    for label, ref in (("local", LOCAL), ("upstream", UPSTREAM)):
        tree = paths(ref)
        target = OUT / label
        write(target / "workspace.tsv", "source\tkind\tvalue", workspace(ref, tree))
        write(target / "rust-public.tsv", "path\tline\tkind\tqualified_name", rust_public(ref, tree))
        write(target / "python-api.tsv", "path\tline\tkind\tqualified_name\tsignature_or_base", python_api(ref, tree))
        test_files, test_names = tests(ref, tree)
        write(target / "test-files.txt", "path", test_files)
        write(target / "test-names.tsv", "path\tline\tname", test_names)
        ledger_rows, claims = conformance(ref, tree)
        write(target / "conformance-rows.tsv", "row_id\tstandard_anchor\tstatus", ledger_rows)
        write(target / "public-support-statements.tsv", "path\tlocation\tstatement", claims)
    write(
        OUT / "local-only-paths.tsv",
        "path\tdisposition\treason",
        classify_local_only(),
    )


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(f"git command failed: {error.cmd}", file=sys.stderr)
        raise SystemExit(error.returncode)
