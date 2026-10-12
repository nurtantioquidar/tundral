#!/usr/bin/env python3
"""Turn SCIP indexes of a corpus repo into ground-truth edges at graph.json granularity.

    uv run --no-project bench/accuracy/oracle.py <repo> [--out PATH]

Reads $DIJKSTRAL_BENCH_HOME/scip/<repo>/parts/*.scip (written by index.sh) through the pinned
`scip print --json`, and the indexed sources (the scratch copy under scip/<repo>/src, or the
corpus checkout for kubernetes). Writes $DIJKSTRAL_BENCH_HOME/accuracy/oracle/<repo>.json:

- `defs`: every non-local SCIP definition (plus function-like locals) as
  [symbol, file, line (1-based), name, kind];
- `edges`: ground truth in symbol space, by family:
    calls       [chain, callee, line]   chain = enclosing function-like defs, innermost first
    references  [chain, type, context]  signature / field / decorator / constructor type mentions
    inherits    [chain, base, kind]     class, interface, trait and impl headers (+ SCIP
                                        is_implementation relationships between types)
    imports     [file, file]            import statements resolving to an in-repo file or symbol
    mentions    [chain, def]            any other in-body mention of an in-repo function or type
                                        (used only to classify false positives, never scored)

Each record keeps the whole chain of enclosing definitions so that score.py can attribute it to
the innermost definition a given tool actually models (Rust nested fns, TS arrow consts inside
functions and closures are not graph nodes in either tool). `map_defs()` maps defs to the nodes
of one tool's graph.json by (file, line, name). Stdlib only.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import subprocess
import sys
from bisect import bisect_right
from collections import defaultdict
from pathlib import Path

BENCH = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)
SCIP = BENCH / "tools/scip/bin/scip"

LANG_BY_EXT = {
    ".py": "py",
    ".pyi": "py",
    ".ts": "ts",
    ".tsx": "ts",
    ".mts": "ts",
    ".cts": "ts",
    ".js": "ts",
    ".jsx": "ts",
    ".mjs": "ts",
    ".cjs": "ts",
    ".rs": "rs",
    ".go": "go",
}
# kinds that become graph nodes; terms only when they hold a function (TS arrow consts)
NODE_KINDS = {"func", "type", "macro"}


# --------------------------------------------------------------------------- SCIP loading


def load_index(path: Path) -> dict:
    out = subprocess.run(
        [str(SCIP), "print", "--json", str(path)], capture_output=True, check=True
    )
    return json.loads(out.stdout)


def symbol_kind(sym: str) -> str:
    if sym.startswith("local "):
        return "local"
    if sym.endswith(")."):
        return "func"
    if sym.endswith("#"):
        return "type"
    if sym.endswith("!"):
        return "macro"
    if sym.endswith("."):
        return "term"
    if sym.endswith("/"):
        return "ns"
    if sym.endswith(":"):
        return "meta"
    if sym.endswith(")"):
        return "param"
    if sym.endswith("]"):
        return "tparam"
    return "other"


def rng(r: list[int]) -> tuple[int, int, int, int]:
    return (r[0], r[1], r[0], r[2]) if len(r) == 3 else (r[0], r[1], r[2], r[3])


class Doc:
    """One source file: text, line offsets and column-unit conversion."""

    def __init__(self, path: str, text: str, unit: str):
        self.path = path
        self.lang = LANG_BY_EXT.get(os.path.splitext(path)[1], "")
        self.text = text
        self.unit = unit
        self.starts = [0]
        for i, ch in enumerate(text):
            if ch == "\n":
                self.starts.append(i + 1)
        self.lines = text.split("\n")

    def off(self, line: int, col: int, unit: str | None = None) -> int:
        if line >= len(self.starts):
            return len(self.text)
        s = self.lines[line]
        u = unit or self.unit
        if col and not s.isascii():
            if u == "utf8":
                col = len(s.encode("utf-8")[:col].decode("utf-8", "ignore"))
            elif u == "utf16":
                col = len(
                    s.encode("utf-16-le")[: 2 * col].decode("utf-16-le", "ignore")
                )
        return self.starts[line] + min(col, len(s))

    def line_of(self, off: int) -> int:
        return bisect_right(self.starts, off) - 1


# --------------------------------------------------------------------------- text scanning


def mask(text: str, lang: str, path: str = "") -> str:
    """Blank out comments and string contents (keeping delimiters) so brackets can be matched."""
    out = list(text)
    n = len(text)
    i = 0
    tmpl: list[int] = []  # TS template-literal brace depth stack
    jsx = lang == "ts" and path.endswith("x")

    def blank(a: int, b: int) -> None:
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    depth = 0
    while i < n:
        c = text[i]
        nx = text[i + 1] if i + 1 < n else ""
        if c == "/" and nx == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
            continue
        if c == "/" and nx == "*":
            if lang == "rs":  # nested block comments
                d, j = 1, i + 2
                while j < n and d:
                    if text.startswith("/*", j):
                        d, j = d + 1, j + 2
                    elif text.startswith("*/", j):
                        d, j = d - 1, j + 2
                    else:
                        j += 1
            else:
                j = text.find("*/", i + 2)
                j = n if j < 0 else j + 2
            blank(i, j)
            i = j
            continue
        if (
            lang == "rs"
            and c == "r"
            and (nx == '"' or nx == "#")
            and (i == 0 or not text[i - 1].isalnum())
        ):
            m = re.match(r'r(#*)"', text[i:])
            if m:
                close = '"' + m.group(1)
                j = text.find(close, i + len(m.group(0)))
                j = n if j < 0 else j + len(close)
                blank(i + len(m.group(0)), j - len(close))
                i = j
                continue
        if lang == "rs" and c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'\n])'", text[i:])
            if m:
                blank(i + 1, i + len(m.group(0)) - 1)
                i += len(m.group(0))
                continue
            i += 1  # lifetime
            continue
        if lang == "ts" and c in "'/" and not expr_position(text, i):
            # JSX text (`Don't`) and division are not string or regex literals
            if c == "/" and nx not in "/*":
                i += 1
                continue
            if c == "'" and jsx:
                i += 1
                continue
        if lang == "ts" and c == "/" and nx not in "/*":
            k = i - 1
            while k >= 0 and text[k] in " \t":
                k -= 1
            if nx == ">" or (k >= 0 and text[k] in "<>}"):  # JSX `</a>`, `/>`
                i += 1
                continue
            j = i + 1
            in_class = False
            while j < n and text[j] != "\n":
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == "[":
                    in_class = True
                elif text[j] == "]":
                    in_class = False
                elif text[j] == "/" and not in_class:
                    break
                j += 1
            if j < n and text[j] == "/":
                blank(i + 1, j)
                i = j + 1
                continue
            i += 1
            continue
        if (
            c == '"'
            or (c == "'" and lang in ("ts", "go"))
            or (c == "`" and lang == "go")
        ):
            j = i + 1
            while j < n and text[j] != c and not (text[j] == "\n" and c != "`"):
                j += 2 if text[j] == "\\" and c != "`" else 1
            blank(i + 1, j)
            i = j + 1
            continue
        if c == "`" and lang == "ts":
            j = i + 1
            while j < n and text[j] != "`":
                if text[j] == "\\":
                    j += 2
                    continue
                if text.startswith("${", j):
                    break
                j += 1
            blank(i + 1, j)
            if j < n and text[j] == "`":
                i = j + 1
            else:  # entering ${ ... }
                tmpl.append(depth)
                depth += 1
                i = j + 2
            continue
        if lang == "ts" and c == "}" and tmpl and depth - 1 == tmpl[-1]:
            depth -= 1
            tmpl.pop()
            # resume template literal
            j = i + 1
            while j < n and text[j] != "`":
                if text[j] == "\\":
                    j += 2
                    continue
                if text.startswith("${", j):
                    break
                j += 1
            blank(i + 1, j)
            if j < n and text[j] == "`":
                i = j + 1
            else:
                tmpl.append(depth)
                depth += 1
                i = j + 2
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return "".join(out)


OPEN, CLOSE = "([{", ")]}"


def expr_position(text: str, i: int) -> bool:
    """Whether text[i] starts an expression operand (so `/` is a regex and `'` a string)."""
    j = i - 1
    while j >= 0 and text[j] in " \t":
        j -= 1
    if j < 0 or text[j] == "\n":
        return True
    if text[j] in "(,=:[!&|?{};+-*%<>~^":
        return True
    k = j
    while k >= 0 and (text[k].isalnum() or text[k] in "_$"):
        k -= 1
    return text[k + 1 : j + 1] in (
        "return",
        "typeof",
        "case",
        "in",
        "of",
        "await",
        "yield",
        "void",
    )


def match(m: str, i: int) -> int:
    """Index of the bracket closing m[i] (one of ([{), or len(m)."""
    stack = [CLOSE[OPEN.index(m[i])]]
    j = i + 1
    n = len(m)
    while j < n and stack:
        c = m[j]
        if c in OPEN:
            stack.append(CLOSE[OPEN.index(c)])
        elif c in CLOSE:
            if c == stack[-1]:
                stack.pop()
            else:  # mismatched closer: give up at this point
                return j
        j += 1
    return j - 1 if not stack else n


def skip_ws(m: str, i: int) -> int:
    while i < len(m) and m[i] in " \t\r\n":
        i += 1
    return i


def skip_angle(m: str, i: int) -> int:
    """m[i] == '<': index after the matching '>' ('->' and '=>' do not close)."""
    d, j, n = 0, i, len(m)
    while j < n:
        c = m[j]
        if c == "<":
            d += 1
        elif c == ">" and m[j - 1] not in "-=":
            d -= 1
            if d == 0:
                return j + 1
        elif c in "([{":
            j = match(m, j)
        elif c in ";)]}":
            return j
        j += 1
    return n


def scan_to(m: str, i: int, stops: tuple[str, ...]) -> int:
    """First index >= i at bracket depth 0 (angle-aware) where one of `stops` starts."""
    n = len(m)
    ang = 0
    while i < n:
        for s in stops:
            if ang == 0 and m.startswith(s, i):
                if s.isalpha() and (
                    m[i - 1 : i].isalnum() or m[i + len(s) : i + len(s) + 1].isalnum()
                ):
                    continue
                return i
        c = m[i]
        if c in "([":
            i = match(m, i) + 1
            continue
        if c == "{" and ("{" not in stops or ang):
            i = match(m, i) + 1
            continue
        if c == "<":
            ang += 1
        elif c == ">" and m[i - 1] not in "-=" and ang:
            ang -= 1
        elif c in ")]}":
            return i
        i += 1
    return n


def angle_depth(m: str, a: int, b: int) -> int:
    d = 0
    for k in range(a, b):
        c = m[k]
        if c == "<":
            d += 1
        elif c == ">" and m[k - 1] not in "-=" and d:
            d -= 1
    return d


# --------------------------------------------------------------------------- spans


class Spans:
    """Typed spans of one document: (start, end, def_index, extra)."""

    def __init__(self) -> None:
        self.fn: list[
            tuple[int, int, int]
        ] = []  # function-like bodies (incl. signature)
        self.sig: list[tuple[int, int, int]] = []  # signature type positions
        self.field: list[
            tuple[int, int, int]
        ] = []  # class/struct bodies, for field types
        self.header: list[tuple[int, int, int, str]] = []  # inheritance bases of def
        self.decorator: list[tuple[int, int, int]] = []
        self.imports: list[tuple[int, int]] = []
        # Rust impl headers: (trait_start, trait_end, type_start, type_end)
        self.impls: list[tuple[int, int, int, int]] = []


class Interval:
    """Static interval lookup: sorted starts + linear filter over candidates."""

    def __init__(self, spans: list):
        self.spans = sorted(spans, key=lambda s: s[0])
        self.starts = [s[0] for s in self.spans]
        self.maxlen = max((s[1] - s[0] for s in self.spans), default=0)

    def find(self, pos: int) -> list:
        if not self.spans:
            return []
        hi = bisect_right(self.starts, pos)
        lo = bisect_right(self.starts, pos - self.maxlen - 1)
        hits = [s for s in self.spans[lo:hi] if s[0] <= pos < s[1]]
        hits.sort(key=lambda s: (-s[0], s[1]))
        return hits


def python_submodule_imports(doc: Doc, files: set[str]) -> set[str]:
    """In-repo files bound by `from pkg import name` when `pkg.name` is a module.

    scip-python records such a name (`from erpnext.x import y as z`) as a local symbol, so the
    occurrence pass cannot see the file it imports. Python's own rule settles it: if `pkg/name.py`
    or `pkg/name/__init__.py` exists, the statement imports that file.
    """
    try:
        tree = ast.parse(doc.text)
    except (SyntaxError, ValueError):
        return set()
    here = doc.path.rsplit("/", 1)[0] if "/" in doc.path else ""
    out: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.ImportFrom):
            continue
        if node.level:
            parts = here.split("/") if here else []
            if node.level - 1 > len(parts):
                continue
            base = parts[: len(parts) - (node.level - 1)]
            pkg = "/".join(base + (node.module.split(".") if node.module else []))
            bases = [pkg]
        elif node.module:
            pkg = node.module.replace(".", "/")
            bases = [pkg, f"src/{pkg}"]
        else:
            continue
        for alias in node.names:
            if alias.name == "*":
                continue
            for b in bases:
                stem = f"{b}/{alias.name}" if b else alias.name
                for cand in (f"{stem}.py", f"{stem}/__init__.py"):
                    if cand in files:
                        out.add(cand)
    return out


def python_spans(doc: Doc, defs_at: dict, sp: Spans) -> bool:
    try:
        tree = ast.parse(doc.text)
    except (SyntaxError, ValueError):
        return False

    def span(node: ast.expr | ast.stmt) -> tuple[int, int]:
        a = doc.off(node.lineno - 1, node.col_offset, "utf8")
        end_line = node.end_lineno if node.end_lineno is not None else node.lineno
        end_col = (
            node.end_col_offset if node.end_col_offset is not None else node.col_offset
        )
        return a, doc.off(end_line - 1, end_col, "utf8")

    def head(e: ast.expr) -> ast.expr | None:
        while isinstance(e, ast.Subscript):
            e = e.value
        return e if isinstance(e, (ast.Name, ast.Attribute)) else None

    def visit(node: ast.AST, cls: int | None) -> None:
        for ch in ast.iter_child_nodes(node):
            if isinstance(ch, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                d = defs_at.get((ch.lineno - 1, ch.name))
                if d is not None:
                    _a, b = span(ch)
                    for dec in ch.decorator_list:
                        # only the decorator itself (`get` in `@app.get(...)`), not its receiver
                        # or arguments
                        h = dec.func if isinstance(dec, ast.Call) else dec
                        a, z = span(h)
                        if isinstance(h, ast.Attribute):
                            a = max(a, z - len(h.attr))
                        sp.decorator.append((a, z, d))
                    if isinstance(ch, ast.ClassDef):
                        for base in ch.bases:
                            h = head(base)
                            if h is not None:
                                sp.header.append((*span(h), d, "inherits"))
                    else:
                        # The body only: decorators, annotations and default values are evaluated
                        # when the def runs, so calls there belong to the enclosing scope.
                        sp.fn.append((span(ch.body[0])[0], b, d))
                        args = ch.args
                        for arg in [
                            *args.posonlyargs,
                            *args.args,
                            *args.kwonlyargs,
                            args.vararg,
                            args.kwarg,
                        ]:
                            if arg is not None and arg.annotation is not None:
                                sp.sig.append((*span(arg.annotation), d))
                        if ch.returns is not None:
                            sp.sig.append((*span(ch.returns), d))
                visit(ch, d if isinstance(ch, ast.ClassDef) else None)
            elif isinstance(ch, (ast.Import, ast.ImportFrom)):
                sp.imports.append(span(ch))
            else:
                visit(ch, cls)

    visit(tree, None)
    # fields: class-level annotations only (`x: Foo` in a class body), not the whole class body
    sp.field = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef):
            d = defs_at.get((node.lineno - 1, node.name))
            if d is None:
                continue
            for st in node.body:
                if isinstance(st, ast.AnnAssign):
                    sp.field.append((*span(st.annotation), d))
    return True


def ts_fnlike(m: str, e: int) -> tuple[int, int] | None:
    """For a term definition ending at e, return (sig_end, body_end) if it holds a function."""
    n = len(m)
    i = skip_ws(m, e)
    if i < n and m[i] in "?!":
        i = skip_ws(m, i + 1)
    if i < n and m[i] == ":":
        i = scan_to(m, i + 1, ("=", ";", "\n"))
        while i < n and m.startswith("=>", i):
            i = scan_to(m, i + 2, ("=", ";", "\n"))
    if i >= n or m[i] != "=" or m.startswith("==", i) or m.startswith("=>", i):
        return None
    i = skip_ws(m, i + 1)
    if m.startswith("async", i) and not m[i + 5 : i + 6].isalnum():
        i = skip_ws(m, i + 5)
    if m.startswith("function", i) and not m[i + 8 : i + 9].isalnum():
        b = scan_to(m, i, ("{",))
        if b >= n or m[b] != "{":
            return None
        return b, match(m, b) + 1
    if i < n and m[i] == "<":
        i = skip_ws(m, skip_angle(m, i))
    if i < n and m[i] == "(":
        j = skip_ws(m, match(m, i) + 1)
        if j < n and m[j] == ":":
            j = scan_to(m, j + 1, ("=>", "{", ";"))
        if not m.startswith("=>", j):
            return None
        arrow = j
    else:
        mm = re.match(r"[A-Za-z_$][\w$]*\s*=>", m[i : i + 200])
        if not mm:
            return None
        arrow = i + mm.end() - 2
    j = skip_ws(m, arrow + 2)
    if j < n and m[j] == "{":
        return arrow, match(m, j) + 1
    return arrow, scan_to(m, j, (";", ","))


def scanner_spans(
    doc: Doc, m: str, defs_in_doc: list[tuple[int, int, int, dict]], sp: Spans
) -> None:
    """TS / Rust / Go spans from the masked text and the SCIP definitions of this document.

    defs_in_doc: (def_index, name_start_off, name_end_off, info) with info = {kind, enc}.
    """
    lang = doc.lang
    n = len(m)
    for d, a, e, info in defs_in_doc:
        kind = info["kind"]
        enc = info.get("enc")
        before = m[max(0, a - 40) : a]
        if (
            lang == "go"
            and kind == "term"
            and m[skip_ws(m, e) : skip_ws(m, e) + 1] == "("
            and "#" in info.get("sym", "")
        ):
            # scip-go names interface methods as terms (`Cache#Dump.`): a method signature
            kind = "func"
            info["fnlike"] = True
        if kind == "func" or (
            kind == "local"
            and lang == "ts"
            and re.search(r"\bfunction\s*\*?\s*$", before)
        ):
            # signature: generics, params, return type up to the body or ';'
            i = skip_ws(m, e)
            if i < n and m[i] in "?!":
                i = skip_ws(m, i + 1)
            if i < n and m[i] == "<":
                i = skip_angle(m, i)
            elif lang == "go" and i < n and m[i] == "[":
                i = match(m, i) + 1
            i = skip_ws(m, i)
            if i >= n or m[i] != "(":
                if enc:
                    sp.fn.append((enc[0], enc[1], d))
                continue
            stops = ("{", ";", "=>") if lang == "ts" else ("{", ";")
            sig_end = scan_to(m, match(m, i) + 1, stops)
            sp.sig.append((e, sig_end, d))
            if enc and enc[1] > e:
                sp.fn.append((min(enc[0], a), enc[1], d))
            elif sig_end < n and m[sig_end] == "{":
                sp.fn.append((a, match(m, sig_end) + 1, d))
            continue
        if lang == "ts" and kind in ("term", "local"):
            r = ts_fnlike(m, e)
            if r is not None:
                info["fnlike"] = True
                sp.sig.append((e, r[0], d))
                sp.fn.append((a, r[1], d))
                continue
            # `const C = memo((props) => …)`, `const f = useCallback(() => …, [])`: calls inside
            # the wrapped anonymous function belong to the binding (anonymous functions are
            # transparent), so the whole initializer is its scope.
            i = skip_ws(m, e)
            if i < n and m[i] == ":":
                i = scan_to(m, i + 1, ("=", ";"))
            if (
                i < n
                and m[i] == "="
                and not m.startswith("==", i)
                and not m.startswith("=>", i)
            ):
                end = scan_to(m, i + 1, (";",))
                if re.search(r"=>|\bfunction\b", m[i:end]):
                    sp.fn.append((a, end, d))
            continue
        if kind != "type":
            continue
        if lang == "ts":
            kw = re.search(r"\b(class|interface)\s+$", before)
            if not kw:
                continue
            i = skip_ws(m, e)
            if i < n and m[i] == "<":
                i = skip_angle(m, i)
            body = scan_to(m, i, ("{",))
            sp.header.append(
                (i, body, d, "class" if kw.group(1) == "class" else "interface")
            )
            if body < n and m[body] == "{":
                sp.field.append((body, match(m, body) + 1, d))
        elif lang == "rs":
            i = skip_ws(m, e)
            if i < n and m[i] == "<":
                i = skip_angle(m, i)
            if re.search(r"\btrait\s+$", before):
                i = skip_ws(m, i)
                if i < n and m[i] == ":":
                    hdr_end = scan_to(m, i + 1, ("{", "where", ";"))
                    sp.header.append((i + 1, hdr_end, d, "inherits"))
                continue
            if re.search(r"\b(struct|enum|union)\s+$", before):
                body = scan_to(m, i, ("{", "(", ";"))
                if body < n and m[body] in "{(":
                    sp.field.append((body, match(m, body) + 1, d))
        elif lang == "go":
            i = skip_ws(m, e)
            if i < n and m[i] == "[":
                i = skip_ws(m, match(m, i) + 1)
            mm = re.match(r"(struct|interface)\s*\{", m[i : i + 40])
            if mm:
                b = i + mm.end() - 1
                sp.field.append((b, match(m, b) + 1, d))

    if lang == "ts":
        # `import "x"`, `import … from "x"`, `export … from "x"` (static module statements only)
        for mm in re.finditer(r"(?m)^[ \t]*(import|export)\b", m):
            j = mm.end()
            while j < n and m[j] not in ";\"'`":
                if m[j] in "([{":
                    j = match(m, j)
                j += 1
            if j < n and m[j] in "\"'":
                pre = m[mm.end() : j]
                if (mm.group(1) == "import" and not pre.strip()) or re.search(
                    r"\bfrom\s*$", pre
                ):
                    q = m.find(m[j], j + 1)
                    sp.imports.append((mm.start(), (q if q > 0 else j) + 1))
    elif lang == "rs":
        for mm in re.finditer(r"(?m)^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?use\s", m):
            j = m.find(";", mm.end())
            sp.imports.append((mm.start(), n if j < 0 else j + 1))
        for mm in re.finditer(r"(?m)^[ \t]*(?:(?:unsafe|default)\s+)*impl\b", m):
            i = skip_ws(m, mm.end())
            if i < n and m[i] == "<":
                i = skip_angle(m, i)
            end = scan_to(m, i, ("{", "where", ";"))
            fo = scan_to(m, i, (" for ", "\nfor ", "{", "where", ";"))
            if fo < end and m[fo] in " \n":
                sp.impls.append((i, fo, fo + 5, end))
    elif lang == "go":
        for mm in re.finditer(r"(?m)^import\b", m):
            j = skip_ws(m, mm.end())
            if j < n and m[j] == "(":
                sp.imports.append((mm.start(), match(m, j) + 1))
            else:
                k = m.find("\n", j)
                sp.imports.append((mm.start(), n if k < 0 else k))


# --------------------------------------------------------------------------- call syntax


def is_construct(doc: Doc, m: str, a: int) -> bool:
    """TS/JS `new X` or a JSX element `<X` (not a generic argument such as `useState<X>`)."""
    pre = m[max(0, a - 40) : a]
    if re.search(r"\bnew\s+$", pre):
        return True
    return doc.path.endswith("x") and bool(
        re.search(r"(?:^|[(\[{},=?:&|>;\n]|\breturn)\s*<\s*$", pre)
    )


def is_call(doc: Doc, m: str, a: int, e: int, target_kind: str) -> bool:
    lang = doc.lang
    n = len(m)
    i = e
    while i < n and m[i] in " \t":
        i += 1
    c = m[i] if i < n else ""
    if lang == "py":
        return c == "("
    if lang == "ts":
        if is_construct(doc, m, a):
            return True
        if target_kind == "type":
            return False
        if c == "(" or c == "`" or m.startswith("?.(", i):
            return True
        if c == "<":
            j = skip_ws(m, skip_angle(m, i))
            return j < n and m[j] == "("
        return False
    if lang == "rs":
        if c in "(!":
            return True
        if m.startswith("::<", i):
            j = skip_ws(m, skip_angle(m, i + 2))
            return j < n and m[j] == "("
        return False
    if lang == "go":
        if c == "(":
            return True
        if c == "[":
            j = match(m, i) + 1
            return j < n and m[j] == "("
    return False


# --------------------------------------------------------------------------- oracle


def source_base(repo: str) -> Path:
    src = BENCH / "scip" / repo / "src"
    return src if src.is_dir() else BENCH / "corpus" / repo


def unit_of(meta: dict) -> str:
    enc = meta.get("text_document_encoding")
    tool = meta.get("tool_info", {}).get("name", "")
    if enc == 1:
        return "utf8"
    if enc == 2:
        return "utf16"
    return "utf16" if "typescript" in tool else "utf8"


def build(repo: str) -> dict:
    parts = sorted((BENCH / "scip" / repo / "parts").glob("*.scip"))
    if not parts:
        sys.exit(f"no SCIP parts for {repo}; run bench/accuracy/index.sh {repo}")
    base = source_base(repo).resolve()
    tools = []
    docs: dict[str, tuple[Doc, list[dict]]] = {}
    rels: list[
        tuple[str, str]
    ] = []  # (type symbol, base symbol) from is_implementation
    for part in parts:
        idx = load_index(part)
        meta = idx.get("metadata", {})
        tools.append({"part": part.stem, **meta.get("tool_info", {})})
        root = Path(meta["project_root"].removeprefix("file://")).resolve()
        unit = unit_of(meta)
        for d in idx.get("documents", []):
            rel = os.path.normpath(
                os.path.join(os.path.relpath(root, base), d["relative_path"])
            )
            if (
                rel.startswith("..")
                or os.path.splitext(rel)[1] not in LANG_BY_EXT
                or rel in docs
            ):
                continue
            try:
                text = (base / rel).read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            docs[rel] = (Doc(rel, text, unit), d.get("occurrences", []))
            for s in d.get("symbols", []):
                for r in s.get("relationships") or []:
                    # Go's implicit interface satisfaction is not an `implements` edge in either
                    # tool's model (§12.2); Python and TS relationships are explicit bases.
                    if r.get("is_implementation") and not rel.endswith(".go"):
                        rels.append((s["symbol"], r["symbol"]))
        del idx

    # ---- definitions
    def_index: dict[str, int] = {}
    defs: list[list] = []  # [symbol, file, line1, name, kind]
    infos: list[dict] = []
    sym_defdocs: dict[str, set[str]] = defaultdict(set)
    doc_defs: dict[str, list[tuple[int, int, int, dict]]] = defaultdict(list)
    for path, (doc, occs) in docs.items():
        for o in occs:
            if not (o.get("symbol_roles", 0) & 1):
                continue
            sym = o["symbol"]
            kind = symbol_kind(sym)
            sl, sc, el, ec = rng(o["range"])
            if kind in ("ns", "meta"):
                if sc == 0 and sl == 0:
                    sym_defdocs[sym].add(path)
                elif kind == "ns" and doc.lang == "go":
                    sym_defdocs[sym].add(path)  # package clause of every file
                continue
            if kind in ("param", "tparam", "other"):
                continue
            key = f"{path}#{sym}" if kind == "local" else sym
            a, e = doc.off(sl, sc), doc.off(el, ec)
            name = doc.text[a:e] if sl == el else ""
            enc = None
            if o.get("enclosing_range"):
                es, esc, ee, eec = rng(o["enclosing_range"])
                enc = (doc.off(es, esc), doc.off(ee, eec))
            if key in def_index:
                continue
            if kind == "local" and doc.lang != "ts":
                continue  # Rust/Go/Python locals are never graph nodes
            if kind == "local" and not re.search(
                r"\b(function\s*\*?|const|let)\s+$", doc.text[max(0, a - 20) : a]
            ):
                continue
            def_index[key] = len(defs)
            defs.append([sym, path, sl + 1, name, kind])
            info = {"kind": kind, "enc": enc, "sym": sym}
            infos.append(info)
            doc_defs[path].append((len(defs) - 1, a, e, info))
            sym_defdocs[sym].add(path)

    # ---- spans and edges
    calls: set[tuple] = set()
    refs: set[tuple] = set()
    inh: set[tuple] = set()
    imports: set[tuple[str, str]] = set()
    mentions: set[tuple] = set()
    py_fallback = 0
    py_files = {p for p, (d, _o) in docs.items() if d.lang == "py"}
    for path, (doc, occs) in docs.items():
        if doc.lang == "py":
            for tdoc in python_submodule_imports(doc, py_files):
                if tdoc != path:
                    imports.add((path, tdoc))
        sp = Spans()
        m = doc.text if doc.lang == "py" else mask(doc.text, doc.lang, doc.path)
        dd = doc_defs.get(path, [])
        if doc.lang == "py":
            at = {}
            for d, a, _e, _info in dd:
                at[(defs[d][2] - 1, defs[d][3])] = d
            if not python_spans(doc, at, sp):
                py_fallback += 1
                for d, a, _e, info in dd:
                    if info["kind"] == "func" and info["enc"]:
                        sp.fn.append((info["enc"][0], info["enc"][1], d))
        else:
            scanner_spans(doc, m, dd, sp)
        for d, _a, _e, info in dd:
            if info.get("fnlike"):
                defs[d][4] = "func"
        fn_iv, sig_iv, field_iv = Interval(sp.fn), Interval(sp.sig), Interval(sp.field)
        hdr_iv, dec_iv, imp_iv = (
            Interval(sp.header),
            Interval(sp.decorator),
            Interval(sp.imports),
        )
        impl_iv = Interval([(a, e, k) for k, (a, _b, _c, e) in enumerate(sp.impls)])
        impl_hits: dict[int, dict[str, list[int]]] = defaultdict(
            lambda: {"trait": [], "type": []}
        )
        for o in occs:
            if o.get("symbol_roles", 0) & 1:
                continue
            sym = o["symbol"]
            kind = symbol_kind(sym)
            sl, sc, el, ec = rng(o["range"])
            a, e = doc.off(sl, sc), doc.off(el, ec)
            if imp_iv.find(a):
                if doc.lang == "rs" and m[skip_ws(m, e) : skip_ws(m, e) + 2] == "::":
                    continue  # `crate::a::B`: only the leaf `B` is a dependency, not the path
                if kind in ("ns", "meta"):
                    targets = sym_defdocs.get(sym, set())
                    if doc.lang == "rs" and len(targets) > 1:
                        # `crate/` is defined by every crate root of a package (lib.rs, main.rs,
                        # each tests/*.rs): the library root is the one a `use` reaches
                        roots = {p for p in targets if p.endswith("src/lib.rs")}
                        targets = roots or targets
                else:
                    t = def_index.get(sym)
                    targets = {defs[t][1]} if t is not None else set()
                for tdoc in targets:
                    if tdoc != path:
                        imports.add((path, tdoc))
                continue
            t = def_index.get(f"{path}#{sym}" if kind == "local" else sym)
            if t is None:
                continue
            tkind = defs[t][4]
            if impl_iv.spans:
                for _s, _e2, k in impl_iv.find(a):
                    ta, tb, ya, yb = sp.impls[k]
                    if tkind == "type" and ta <= a < tb and angle_depth(m, ta, a) == 0:
                        impl_hits[k]["trait"].append(t)
                    elif (
                        tkind == "type" and ya <= a < yb and angle_depth(m, ya, a) == 0
                    ):
                        impl_hits[k]["type"].append(t)
                if impl_iv.find(a):
                    continue
            chain = tuple(s[2] for s in fn_iv.find(a))
            dec = dec_iv.find(a)
            if dec:
                if tkind in NODE_KINDS:
                    refs.add(((dec[0][2],), t, "decorator"))
                    if chain:  # applying a decorator inside a function calls it there
                        calls.add((chain, t, sl + 1))
                continue
            hdr = hdr_iv.find(a)
            if hdr and tkind == "type":
                h = hdr[0]
                if doc.lang == "ts" and angle_depth(m, h[0], a) != 0:
                    pass
                else:
                    rel = h[3]
                    if doc.lang == "ts":
                        kw = re.findall(r"\b(extends|implements)\b", m[h[0] : a])
                        rel = (
                            "implements"
                            if kw and kw[-1] == "implements"
                            else "inherits"
                        )
                    inh.add(((h[2],), t, rel))
                    continue
            sig = sig_iv.find(a)
            if sig and tkind in NODE_KINDS:
                refs.add(((sig[0][2],), t, "signature"))
                # Python `Annotated[T, Body()]`: the annotation runs in the enclosing scope
                if doc.lang == "py" and chain and is_call(doc, m, a, e, tkind):
                    calls.add((chain, t, sl + 1))
                continue
            if chain:
                call = tkind in ("func", "macro", "local", "term") and is_call(
                    doc, m, a, e, tkind
                )
                if not call and tkind == "type":
                    # Python `Cls(...)` and TS `new Cls` / `<Cls/>` construct an instance (a call to
                    # the class node); Rust `Tuple(..)` / `Enum::Variant(..)` are constructor
                    # references; Go `T(x)` is a conversion, not a call.
                    if doc.lang in ("py", "ts"):
                        call = is_call(doc, m, a, e, tkind)
                    elif doc.lang == "rs" and is_call(doc, m, a, e, tkind):
                        refs.add((chain, t, "constructor"))
                        continue
                if call:
                    calls.add((chain, t, sl + 1))
                elif tkind in NODE_KINDS or tkind in ("local", "term"):
                    mentions.add((chain, t))
                continue
            fld = field_iv.find(a)
            if fld and tkind == "type":
                if doc.lang == "ts" and not re.search(
                    r"[:<,|&\[(]\s*(?:[\w$]+\s*\.\s*)*$", m[max(0, a - 80) : a]
                ):
                    continue
                refs.add(((fld[0][2],), t, "field"))
        for k, hit in impl_hits.items():
            if hit["trait"] and hit["type"]:
                inh.add(((hit["type"][0],), hit["trait"][-1], "implements"))
    for sub, sup in rels:
        a, b = def_index.get(sub), def_index.get(sup)
        if (
            a is not None
            and b is not None
            and defs[a][4] == "type"
            and defs[b][4] == "type"
            and a != b
        ):
            inh.add(((a,), b, "inherits"))

    # keep the first call line per (chain, callee)
    first: dict[tuple, int] = {}
    for chain, t, line in calls:
        if (chain, t) not in first or line < first[(chain, t)]:
            first[(chain, t)] = line
    return {
        "repo": repo,
        "commit": (BENCH / "scip" / repo / "COMMIT").read_text().strip()
        if (BENCH / "scip" / repo / "COMMIT").exists()
        else "",
        "indexers": tools,
        "docs": sorted(docs),
        "python_ast_fallback_docs": py_fallback,
        "defs": defs,
        "edges": {
            "calls": sorted([list(c), t, ln] for (c, t), ln in first.items()),
            "references": sorted([list(c), t, ctx] for c, t, ctx in refs),
            "inherits": sorted([list(c), t, k] for c, t, k in inh),
            "imports": sorted([a, b] for a, b in imports),
            "mentions": sorted([list(c), t] for c, t in mentions),
        },
    }


# --------------------------------------------------------------------------- mapping


def label_name(label: str) -> str:
    s = label.strip().removeprefix(".")
    for suf in ("()", "!"):
        s = s.removesuffix(suf)
    for ch in "<[(":
        k = s.find(ch)
        if k > 0:
            s = s[:k]
    return s


def is_file_node(n: dict) -> bool:
    sf = n.get("source_file") or ""
    return (
        bool(sf)
        and n.get("source_location") in ("L1", None)
        and n["label"].endswith(os.path.basename(sf))
    )


def map_defs(
    oracle: dict, nodes: list[dict], window: int = 3
) -> tuple[dict[int, str], dict]:
    """Map oracle defs to one tool's graph nodes by (file, line, name).

    Exact (file, line) with a matching label first; otherwise the nearest node within `window`
    lines with a matching label (decorators, attributes, multi-line headers). Returns
    {def_index: node_id} and per-kind statistics.
    """
    by_file_line: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for n in nodes:
        sf, loc = n.get("source_file") or "", n.get("source_location") or ""
        if (
            not sf
            or not loc.startswith("L")
            or n.get("file_type") not in (None, "code")
        ):
            continue
        if is_file_node(n) and n["label"] != label_name(n["label"]):
            continue
        by_file_line[(sf, int(loc[1:]))].append(n)
    out: dict[int, str] = {}
    stats: dict[str, dict[str, int]] = defaultdict(
        lambda: {"defs": 0, "exact": 0, "shifted": 0}
    )
    for i, (_sym, path, line, name, kind) in enumerate(oracle["defs"]):
        group = (
            "local"
            if _sym.startswith("local ")
            else kind
            if kind in NODE_KINDS
            else "other"
        )
        stats[group]["defs"] += 1
        if not name:
            continue
        hit = None
        for n in by_file_line.get((path, line), []):
            if label_name(n["label"]) == name:
                hit = n
                break
        if hit is not None:
            out[i] = hit["id"]
            stats[group]["exact"] += 1
            continue
        for dl in sorted(range(-window, window + 1), key=abs):
            if dl == 0:
                continue
            for n in by_file_line.get((path, line + dl), []):
                if label_name(n["label"]) == name:
                    hit = n
                    break
            if hit is not None:
                break
        if hit is not None:
            out[i] = hit["id"]
            stats[group]["shifted"] += 1
    return out, dict(stats)


def oracle_path(repo: str) -> Path:
    return BENCH / "accuracy" / "oracle" / f"{repo}.json"


def load_or_build(repo: str, rebuild: bool = False) -> dict:
    p = oracle_path(repo)
    if p.exists() and not rebuild:
        return json.loads(p.read_text())
    o = build(repo)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(o))
    return o


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("repo")
    ap.add_argument(
        "--out", help="default: $DIJKSTRAL_BENCH_HOME/accuracy/oracle/<repo>.json"
    )
    args = ap.parse_args()
    o = build(args.repo)
    out = Path(args.out) if args.out else oracle_path(args.repo)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(o))
    kinds = defaultdict(int)
    for d in o["defs"]:
        kinds[d[4]] += 1
    print(f"{args.repo}: {len(o['docs'])} docs, {len(o['defs'])} defs {dict(kinds)}")
    for fam, es in o["edges"].items():
        print(f"  {fam:11} {len(es)}")
    if o["python_ast_fallback_docs"]:
        print(
            f"  python files that failed to parse (SCIP ranges used): {o['python_ast_fallback_docs']}"
        )
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
