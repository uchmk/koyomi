#!/usr/bin/env python3
"""No sentence for a person is written in the Rust sources: they live in the message catalogue and the language tables.

python3 scripts/text-in-source.py             counts, by file
python3 scripts/text-in-source.py <path>      the lines left in the files whose path contains <path>
python3 scripts/text-in-source.py --check     exits 1 when anything is left

A line counts when it holds Japanese outside a comment. A `#[cfg(test)]` module does not count. A word the program reads
rather than says goes in scripts/text-in-source.allow as `<path> <word> <reason>`; a line without a reason does not count.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
JAPANESE = re.compile(r"[぀-ヿ㐀-鿿！-｠]")
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*\n\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s")


def allowed() -> dict[str, list[str]]:
    table: dict[str, list[str]] = {}
    path = ROOT / "scripts" / "text-in-source.allow"
    if not path.exists():
        return table
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and not line.startswith("#"):
            table.setdefault(parts[0], []).append(parts[1])
    return table


def code_of(line: str) -> str:
    """The line without its comment. ` // ` inside a string is rare enough to count as text and be looked at."""
    s = line.strip()
    if s.startswith("//"):
        return ""
    return s.split(" // ")[0]


def left(path: pathlib.Path, words: list[str]) -> list[tuple[int, str]]:
    text = path.read_text(encoding="utf-8")
    m = TEST_MODULE.search(text)
    if m:
        text = text[: m.start()]
    out = []
    test_indent = None
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())
        if stripped == "#[cfg(test)]":
            test_indent = indent
            continue
        if test_indent is not None:
            # A test-only item (a const list, a helper, a method): to its end, the first line at the attribute's own indent that closes it.
            if indent == test_indent and stripped.endswith(("}", ";")):
                test_indent = None
            continue
        code = code_of(line)
        for w in words:
            code = code.replace(w, "")
        if JAPANESE.search(code):
            out.append((n, line.strip()))
    return out


def main() -> int:
    args = sys.argv[1:]
    check = "--check" in args
    needle = next((a.replace("\\", "/") for a in args if not a.startswith("--")), None)
    table = allowed()
    rows = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        if "/src/" not in rel:
            continue
        hits = left(path, table.get(rel, []))
        if hits:
            rows.append((rel, hits))
    total = sum(len(h) for _, h in rows)
    for rel, hits in sorted(rows, key=lambda r: -len(r[1])):
        if needle is None:
            if not check:
                print(f"{len(hits):5} {rel}")
        elif needle in rel:
            print(rel)
            for n, line in hits:
                print(f"  {n}: {line[:150]}")
    if check and total:
        for rel, hits in rows:
            for n, line in hits:
                print(f"{rel}:{n}: {line[:150]}")
        print(f"text-in-source: {total} lines hold text for a person; move them to the catalogue or a language table")
        return 1
    if needle is None:
        print(f"total {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
