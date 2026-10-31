#!/usr/bin/env python3
"""Fetch latin woff2 subsets from the Google Fonts CSS2 API for self-hosting.

Tauri bundles the app, so the renderer must not depend on fonts.googleapis.com.
For each family/weight we keep only the final @font-face block (the `latin`
subset is emitted last) and save the woff2 into src/renderer/assets/fonts.
"""
import os
import re
import sys
import urllib.request

OUT = os.path.join("src", "renderer", "assets", "fonts")
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/124.0 Safari/537.36")

# (family, weights, italic) -> output file stem
WANTED = [
    ("Inter", [400, 500, 600, 700], False),
    ("JetBrains Mono", [500, 700], False),
    # Title-style picker fonts (must keep working with no network)
    ("Roboto", [700, 900], False),
    ("Outfit", [700], False),
    ("Oswald", [700], False),
    ("Cinzel", [700], False),
    ("Bebas Neue", [400], False),
    ("Press Start 2P", [400], False),
    ("Playfair Display", [700], False),
    ("Playfair Display", [700], True),
]


def slug(name):
    return name.lower().replace(" ", "-")


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read()


def main():
    os.makedirs(OUT, exist_ok=True)
    written = []
    failed = []
    for family, weights, italic in WANTED:
        for weight in weights:
            fam = family.replace(" ", "+")
            it = "ital,wght@1," if italic else "wght@"
            css_url = (f"https://fonts.googleapis.com/css2?family={fam}:{it}{weight}"
                       f"&display=swap")
            try:
                css = fetch(css_url).decode("utf-8")
            except Exception as exc:  # noqa: BLE001 - report and continue
                failed.append((family, weight, str(exc)))
                continue
            blocks = re.findall(r"@font-face\s*\{[^}]*\}", css)
            if not blocks:
                failed.append((family, weight, "no @font-face in response"))
                continue
            latin = blocks[-1]  # `latin` subset is emitted last
            m = re.search(r"url\((https://[^)]+\.woff2)\)", latin)
            if not m:
                failed.append((family, weight, "no woff2 url"))
                continue
            data = fetch(m.group(1))
            stem = f"{slug(family)}-{weight}"
            if italic:
                stem += "-italic"
            path = os.path.join(OUT, stem + ".woff2")
            with open(path, "wb") as fh:
                fh.write(data)
            written.append((stem, len(data)))

    for stem, size in written:
        print(f"saved {stem}.woff2  {size/1024:.1f} KB")
    for family, weight, err in failed:
        print(f"FAILED {family} {weight}: {err}", file=sys.stderr)
    print(f"\n{len(written)} written, {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
