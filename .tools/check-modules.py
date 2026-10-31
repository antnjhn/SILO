#!/usr/bin/env python3
"""Static sanity check for the renderer ES modules (src/renderer/js).

Reports, precisely:
  * imports of files that do not exist
  * imported names the target module does not export
  * duplicate named exports within a module
  * identifier uses that the module never declares or imports (missing import /
    use-before-definition hint)

Checks 1-3 are exact; the last is a heuristic (regex based, no real parser) and
is reported separately. Exit code is non-zero when problems are found so the
script can gate a build.
"""
import glob
import os
import re
import sys

SRC = "src/renderer/js"

KEYWORDS = {
    "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "export", "extends", "finally", "for",
    "function", "if", "import", "in", "instanceof", "let", "new", "return",
    "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while",
    "with", "yield", "async", "await", "of", "as", "from", "static", "get", "set",
    # globals
    "window", "document", "console", "navigator", "performance", "localStorage",
    "requestAnimationFrame", "cancelAnimationFrame", "setTimeout", "clearTimeout",
    "setInterval", "clearInterval", "Image", "Map", "Set", "WeakMap", "Promise",
    "Date", "Math", "Number", "String", "Boolean", "Array", "Object", "JSON",
    "RegExp", "URL", "Error", "TypeError", "isNaN", "parseInt", "parseFloat",
    "fetch", "undefined", "NaN", "Infinity", "globalThis", "arguments",
    "true", "false", "null", "innerWidth", "innerHeight", "Event",
    "MutationObserver", "ImageBitmap", "AbortController", "TextEncoder",
}

IDENT = re.compile(r"[A-Za-z_$][A-Za-z0-9_$]*")


def strip_comments_and_strings(text):
    """Blank out comments and string/template bodies so scanning sees code only."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = text.find("\n", i)
            i = n if j == -1 else j
        elif c == "/" and nxt == "*":
            j = text.find("*/", i + 2)
            i = n if j == -1 else j + 2
        elif c in "\"'`":
            quote = c
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\":
                    i += 2
                    continue
                if text[i] == quote:
                    i += 1
                    break
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def parse_exports(text):
    names = set()
    dupes = []
    for m in re.finditer(r"export\s+(?:async\s+)?(?:function|const|let|var|class)\s+([A-Za-z_$][\w$]*)", text):
        names.add(m.group(1))
    for m in re.finditer(r"export\s*\{([^}]*)\}", text):
        for part in m.group(1).split(","):
            part = part.strip()
            if not part:
                continue
            if " as " in part:
                part = part.split(" as ")[-1].strip()
            names.add(part)
    seen = {}
    for m in re.finditer(r"^export\s+(?:async\s+)?(?:function|const|let|var|class)\s+([A-Za-z_$][\w$]*)", text, re.M):
        seen[m.group(1)] = seen.get(m.group(1), 0) + 1
    dupes = [k for k, v in seen.items() if v > 1]
    return names, dupes


def parse_imports(text):
    imports = []
    for m in re.finditer(r"import\s*\{([^}]*)\}\s*from\s*['\"]([^'\"]+)['\"]", text):
        names = [p.strip() for p in m.group(1).split(",") if p.strip()]
        imports.append((m.group(2), names))
    for m in re.finditer(r"import\s+([A-Za-z_$][\w$]*)\s+from\s*['\"]([^'\"]+)['\"]", text):
        imports.append((m.group(2), [m.group(1)]))
    return imports


def declared_names(text):
    names = set()
    for m in re.finditer(r"\b(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)", text):
        names.add(m.group(1))
    for m in re.finditer(r"\b(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)", text):
        names.add(m.group(1))
    for m in re.finditer(r"\b(?:export\s+)?class\s+([A-Za-z_$][\w$]*)", text):
        names.add(m.group(1))
    # function/arrow params, catch, for-of / for-in bindings
    for m in re.finditer(r"\(([^()]*)\)\s*(?:=>|\{)", text):
        for p in m.group(1).split(","):
            p = p.strip().lstrip(".").split("=")[0].strip()
            if re.fullmatch(r"[A-Za-z_$][\w$]*", p):
                names.add(p)
    for m in re.finditer(r"([A-Za-z_$][\w$]*)\s*=>", text):
        names.add(m.group(1))
    for m in re.finditer(r"\bcatch\s*\(([A-Za-z_$][\w$]*)\)", text):
        names.add(m.group(1))
    for m in re.finditer(r"\bfor\s*\(\s*(?:const|let|var)\s+([A-Za-z_$][\w$]*)", text):
        names.add(m.group(1))
    # destructuring: const { a, b } = ... / const [a, b] = ...
    for m in re.finditer(r"\b(?:const|let|var)\s*[\{\[]([^\}\]]*)[\}\]]", text):
        for part in m.group(1).split(","):
            part = part.strip().split(":")[0].strip().split("=")[0].strip()
            if re.fullmatch(r"[A-Za-z_$][\w$]*", part):
                names.add(part)
    return names


def main():
    files = sorted(glob.glob(os.path.join(SRC, "*.js")))
    stripped = {}
    exports = {}
    for path in files:
        raw = open(path, encoding="utf-8").read()
        stripped[path] = strip_comments_and_strings(raw)
        exports[os.path.normpath(path)], _ = parse_exports(raw)

    hard = 0
    for path in files:
        raw = open(path, encoding="utf-8").read()
        name = os.path.basename(path)

        for target, imported in parse_imports(raw):
            target_path = os.path.normpath(os.path.join(os.path.dirname(path), target))  # noqa: E501
            if not os.path.exists(target_path):
                print(f"[missing file]  {name}: imports {target}")
                hard += 1
                continue
            available = exports.get(target_path, set())
            for n in imported:
                if n not in available:
                    print(f"[missing export] {name}: '{n}' is not exported by {os.path.basename(target_path)}")
                    hard += 1

        _, dupes = parse_exports(raw)
        for d in dupes:
            print(f"[duplicate export] {name}: '{d}'")
            hard += 1

    # Heuristic: identifier used but not declared/imported in that module.
    soft = 0
    for path in files:
        text = stripped[path]
        name = os.path.basename(path)
        imported_names = set()
        for _t, imported in parse_imports(open(path, encoding="utf-8").read()):
            imported_names.update(imported)
        known = declared_names(text) | imported_names | KEYWORDS
        unknown = {}
        for m in re.finditer(r"(?<![\w$.])([A-Za-z_$][\w$]*)\s*\(", text):
            nm = m.group(1)
            if nm not in known:
                unknown[nm] = unknown.get(nm, 0) + 1
        for nm in sorted(unknown):
            print(f"[undeclared?]  {name}: '{nm}' called but not declared/imported here")
            soft += 1

    if hard == 0 and soft == 0:
        print("module check: OK")
    else:
        print(f"module check: {hard} definite problem(s), {soft} heuristic warning(s)")
    return 1 if hard else 0


if __name__ == "__main__":
    sys.exit(main())
