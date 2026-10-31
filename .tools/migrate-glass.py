#!/usr/bin/env python3
"""Glassmorphism -> flat-surface migration for style.css (line-based, CRLF-safe).

Operates on logical lines so multi-line regexes cannot eat neighbouring
declarations. The file's CRLF convention is preserved on write.
"""
import re
import sys

PATH = "src/renderer/style.css"

with open(PATH, "rb") as fh:
    raw = fh.read().decode("utf-8")
had_crlf = "\r\n" in raw
lines = raw.replace("\r\n", "\n").split("\n")

out = []
report = {"backdrop-filter lines": 0, "glass-shadow lines": 0, "corrupt lines": 0}

for line in lines:
    stripped = line.strip()

    # 1. Drop every backdrop-filter declaration: that IS the glassmorphism.
    if stripped.startswith("backdrop-filter:") or stripped.startswith("-webkit-backdrop-filter:"):
        report["backdrop-filter lines"] += 1
        continue

    # 2. Drop heavy glass drop-shadows (replaced by 1px hairlines).
    if stripped in ("box-shadow: var(--glass-shadow);",):
        report["glass-shadow lines"] += 1
        continue

    # 3. Drop the three corrupted duplicate rules (stray "i " selector prefix).
    if re.match(r"^i\s+(#uninstall-modal|#stat-popover)\b", stripped):
        report["corrupt lines"] += 1
        continue

    # 4. Variable migration to flat tokens.
    line = line.replace("var(--glass-bg)", "var(--surface-1)")
    line = line.replace("var(--glass-border)", "var(--hairline)")
    line = line.replace("var(--glass-highlight)", "var(--hairline-strong)")
    out.append(line)

css = "\n".join(out)

# 5. Every modal/overlay dimmer becomes ONE flat scrim (no blur, no tint).
def scrim(m):
    return m.group(1) + "var(--scrim)"

css, n_scrim = re.subn(
    r"(background:\s*)rgba\((?:1[0-9]|2[0-9]|0|[0-9]),[^)]*0\.[3-8][0-9]?\)",
    scrim,
    css,
)
report["flat scrims"] = n_scrim

if had_crlf:
    css = css.replace("\n", "\r\n")

with open(PATH, "wb") as fh:
    fh.write(css.encode("utf-8"))

for key, value in report.items():
    print(f"{key}: {value}")
leftover = [ln.strip() for ln in css.split("\n") if "--glass-" in ln]
print(f"remaining --glass-* variable refs: {len(leftover)}")
for ln in leftover[:8]:
    print("   " + ln[:120])
sys.exit(0)
