#!/usr/bin/env python3
"""Fail when a Rust diff is mostly mechanical reformatting.

Why this exists
---------------
A reformatting sweep that rides along inside a feature PR makes the change
unreviewable: 1,700 lines of indentation hide 40 lines of real edits. Neither
existing tool catches it:

  * `cargo fmt --check` only says whether the tree is formatted, not whether the
    *diff* is formatting.
  * `git diff -w` ignores whitespace but not re-wrapping. rustfmt splits and
    joins lines, and that shows up as ordinary added/removed lines.

How it works
------------
Format both sides of the diff with rustfmt and compare *those*. rustfmt erases
indentation, line wrapping, blank-line whitespace and trailing whitespace, so
what survives is the semantic change. A large raw diff with a tiny normalized
diff means the PR is a formatting sweep wearing a feature's clothes.

Both counts come from the same pipeline so the ratio is meaningful:

    raw = changed lines between the two sides as committed
    sem = changed lines between rustfmt(base) and rustfmt(head)

The rustfmt config is read from the *base* revision, so narrowing `max_width`
and reformatting the tree in one PR reads as churn rather than as an
unavoidable consequence.

Exit codes: 0 = pass, 1 = fail (or the tool could not run), 0 = skip when there
is nothing to compare.

Usage:
    python3 .tools/check-format-churn.py --base <rev> --head <rev>
"""

from __future__ import annotations

import argparse
import difflib
import re
import subprocess
import sys

DEFAULT_MIN_LINES = 150
DEFAULT_MAX_RATIO = 0.35
# Per-file nudges: not fatal on their own, but a reviewer wants to know.
NOISY_FILE_LINES = 100
NOISY_FILE_RATIO = 0.20


def run(args: list[str], stdin: str | None = None) -> subprocess.CompletedProcess:
    # UTF-8 explicitly: source carries em dashes and other non-ASCII text, and
    # the locale default on Windows runners is cp1252. Decoding git's output
    # through the wrong codec mangles it, and rustfmt then rejects the file as
    # invalid UTF-8 — which silently sends the file down the "unformatted"
    # fallback and under-reports churn.
    return subprocess.run(
        args,
        input=stdin,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def git(*args: str) -> str | None:
    """Run git, returning stdout, or None when the command fails."""
    result = run(["git", *args])
    return result.stdout if result.returncode == 0 else None


def normalize(text: str) -> str:
    """Fold away everything that is not code.

    Removes CRLF, blank-line whitespace and trailing whitespace. Doing this
    before rustfmt keeps rustfmt from choking on whitespace-only lines (it
    reports "left behind trailing whitespace" and refuses to format), and makes
    a pure line-ending or trailing-whitespace sweep read as churn.
    """
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    lines = ["" if line.strip() == "" else line.rstrip() for line in text.split("\n")]
    body = "\n".join(lines)
    # An absent file (added/deleted side) must normalize to nothing, so both the
    # raw and the rustfmt-normalized counts agree that nothing was there.
    return "" if body.strip() == "" else body + "\n"


def changed_lines(before: str, after: str) -> int:
    """Count the +/- lines a reviewer would see in a unified diff."""
    diff = difflib.unified_diff(
        before.splitlines(), after.splitlines(), n=0, lineterm=""
    )
    return sum(
        1
        for line in diff
        if (line.startswith("+") or line.startswith("-"))
        and not line.startswith("+++")
        and not line.startswith("---")
    )


def edition_of(rev: str) -> str:
    manifest = git("show", f"{rev}:src-tauri/Cargo.toml")
    if manifest:
        match = re.search(r'^\s*edition\s*=\s*"([^"]+)"', manifest, re.MULTILINE)
        if match:
            return match.group(1)
    return "2021"


def rustfmt_config(rev: str) -> str:
    """Build a rustfmt config string from the base revision's rustfmt.toml.

    `skip_children` is forced on: we feed rustfmt single files on stdin, so it
    must never try to resolve `mod` declarations and reach outside what we gave
    it.
    """
    options = {"skip_children": "true"}
    for name in ("rustfmt.toml", ".rustfmt.toml"):
        text = git("show", f"{rev}:src-tauri/{name}")
        if not text:
            continue
        for line in text.splitlines():
            line = line.split("#", 1)[0].strip()
            if "=" not in line:
                continue
            key, value = (part.strip() for part in line.split("=", 1))
            if key and value:
                options[key] = value.strip('"')
        break
    return ",".join(f"{key}={value}" for key, value in options.items())


def format_rust(text: str, edition: str, config: str, rustfmt: str) -> tuple[str, bool]:
    """Return (formatted, ok). Falls back to the input when rustfmt cannot run.

    Falling back keeps a rustfmt hiccup from inventing churn: if one side comes
    back unformatted and the other does not, the normalized diff grows and the
    file simply stops counting as mechanical churn.
    """
    # An empty side just means the file was added (or deleted); there is nothing
    # to format, and rustfmt's empty stdout must not look like a failure.
    if not text.strip():
        return "", True
    result = run(
        [rustfmt, "--edition", edition, "--emit", "stdout", "--config", config],
        stdin=text,
    )
    if result.returncode == 0 and result.stdout.strip():
        return normalize(result.stdout), True
    return text, False


def changed_rust_files(base: str, head: str, paths: list[str]) -> list[str]:
    numstat = git("diff", "--numstat", base, head, "--", *paths)
    if not numstat:
        return []
    files: list[str] = []
    for line in numstat.splitlines():
        parts = line.split("\t")
        if len(parts) != 3:
            continue
        added, deleted, path = parts
        # Binary entries report "-"; only Rust source is in scope.
        if added == "-" or not path.endswith(".rs"):
            continue
        files.append(path)
    return files


def file_text(rev: str, path: str) -> str:
    """A file's content at a revision; missing (added/deleted) reads as empty."""
    text = git("show", f"{rev}:{path}")
    return text if text is not None else ""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="base revision (e.g. a merge base)")
    parser.add_argument("--head", default="HEAD", help="head revision (default HEAD)")
    parser.add_argument("--paths", nargs="+", default=["src-tauri"], help="pathspecs")
    parser.add_argument("--min-lines", type=int, default=DEFAULT_MIN_LINES)
    parser.add_argument("--max-ratio", type=float, default=DEFAULT_MAX_RATIO)
    parser.add_argument("--rustfmt", default="rustfmt")
    parser.add_argument(
        "--annotate",
        action="store_true",
        help="emit GitHub ::warning annotations for individual noisy files",
    )
    args = parser.parse_args()

    if not git("rev-parse", "--verify", f"{args.base}^{{commit}}"):
        print(f"cannot resolve base revision {args.base!r}; skipping churn check")
        return 0

    files = changed_rust_files(args.base, args.head, args.paths)
    if not files:
        print("no Rust files changed; nothing to check")
        return 0

    edition = edition_of(args.base)
    config = rustfmt_config(args.base)

    rows: list[tuple[str, int, int, bool]] = []
    for path in files:
        base_text = normalize(file_text(args.base, path))
        head_text = normalize(file_text(args.head, path))
        raw = changed_lines(base_text, head_text)
        if raw == 0:
            continue
        base_fmt, base_ok = format_rust(base_text, edition, config, args.rustfmt)
        head_fmt, head_ok = format_rust(head_text, edition, config, args.rustfmt)
        sem = changed_lines(base_fmt, head_fmt)
        rows.append((path, raw, sem, base_ok and head_ok))

    if not rows:
        print("no Rust changes to measure; nothing to check")
        return 0

    rows.sort(key=lambda row: row[1], reverse=True)
    raw_total = sum(row[1] for row in rows)
    sem_total = sum(row[2] for row in rows)
    ratio = sem_total / raw_total if raw_total else 1.0
    unformatted = [row[0] for row in rows if not row[3]]

    print("### Rust diff vs rustfmt-normalized diff")
    print()
    print("| file | raw lines | semantic lines | formatting |")
    print("| --- | ---: | ---: | ---: |")
    for path, raw, sem, _ in rows:
        share = 0.0 if raw == 0 else max(0.0, 1.0 - sem / raw)
        print(f"| `{path}` | {raw} | {sem} | {share:.0%} |")
    print(f"| **total** | **{raw_total}** | **{sem_total}** | **{max(0.0, 1.0 - ratio):.0%}** |")
    print()
    print(f"Threshold: fail when raw >= {args.min_lines} lines and "
          f"formatting >= {1 - args.max_ratio:.0%} of the diff.")

    if unformatted:
        print()
        print("Note: rustfmt did not produce output for "
              f"{', '.join(f'`{p}`' for p in unformatted)}; those files are "
              "compared unformatted, which can only under-report churn.")

    if args.annotate:
        for path, raw, sem, ok in rows:
            if ok and raw >= NOISY_FILE_LINES and sem / raw <= NOISY_FILE_RATIO:
                print(
                    f"::warning file={path},line=1::"
                    f"{raw - sem} of {raw} changed lines in this file are formatting"
                )

    would_fail = raw_total >= args.min_lines and ratio <= args.max_ratio

    print()
    if would_fail:
        print(
            f"FAIL: {raw_total} lines changed, but only {sem_total} survive rustfmt "
            f"({1 - ratio:.0%} of this diff is reformatting)."
        )
        print(
            "Split the formatting out of the change, or re-run with the "
            "'formatting-sweep' label / an [allow-format-churn] marker in the PR body "
            "if the sweep is the point of the PR."
        )
        return 1

    print(f"OK: {sem_total} of {raw_total} changed lines survive rustfmt "
          f"({max(0.0, 1.0 - ratio):.0%} formatting).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
