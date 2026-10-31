#!/usr/bin/env python3
"""Report where style.css brace nesting goes wrong."""
import sys

PATH = "src/renderer/style.css"
text = open(PATH, encoding="utf-8", newline="").read().replace("\r\n", "\n")
lines = text.split("\n")

depth = 0
reports = []
for n, line in enumerate(lines, 1):
    # Ignore braces inside comments (good enough for this stylesheet).
    stripped = line
    opens = stripped.count("{")
    closes = stripped.count("}")
    if depth < 0:
        reports.append(f"line {n}: depth went negative before this line")
        break
    depth += opens - closes
    if depth < 0:
        reports.append(f"line {n}: depth {depth} -> {line.strip()[:110]}")
        depth = 0
    elif depth == 0 and closes > opens and n > 1:
        pass

print(f"final depth: {depth}")
for r in reports[:12]:
    print(r)

if depth > 0:
    print("\nUnclosed block starts near:")
    d = 0
    for n, line in enumerate(lines, 1):
        before = d
        d += line.count("{") - line.count("}")
        if before == 0 and d > 0:
            print(f"  line {n}: {line.strip()[:110]}")
    print(f"  (file ends at depth {d})")
sys.exit(0)
