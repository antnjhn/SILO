#!/usr/bin/env python3
"""Build the browser harness in .uitest: copy the renderer plus the Tauri stub
(.tools/harness-stub.js) and inject that stub before the stylesheets."""
import os
import shutil

SRC = os.path.join("src", "renderer")
STUB = os.path.join(".tools", "harness-stub.js")
OUT = ".uitest"

if os.path.isdir(OUT):
    shutil.rmtree(OUT)
os.makedirs(OUT, exist_ok=True)

for name in ("style.css", "tokens.css"):
    shutil.copy(os.path.join(SRC, name), os.path.join(OUT, name))
shutil.copytree(os.path.join(SRC, "js"), os.path.join(OUT, "js"))
shutil.copytree(os.path.join(SRC, "assets"), os.path.join(OUT, "assets"))
shutil.copy(STUB, os.path.join(OUT, "stub.js"))

with open(os.path.join(SRC, "index.html"), encoding="utf-8") as fh:
    html = fh.read()

marker = '<link rel="stylesheet" href="tokens.css" />'
assert marker in html, "tokens.css link not found in index.html"
html = html.replace(marker, '<script src="stub.js"></script>\n  ' + marker, 1)

with open(os.path.join(OUT, "index.html"), "w", encoding="utf-8", newline="") as fh:
    fh.write(html)

print("harness ready in", OUT, "| stub injected:", "stub.js" in html)
