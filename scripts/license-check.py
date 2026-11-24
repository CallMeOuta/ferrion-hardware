#!/usr/bin/env python3
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

TREE = "drivers"
RECORD = f"{TREE}/upstream.toml"
OURS = "GPL-2.0-only"
INHERITED = "GPL-2.0-only AND MIT"
TAG = "SPDX-License-Identifier: "
STYLES = {".rs": ("// ", ""), ".toml": ("# ", ""), ".gitignore": ("# ", ""), ".sh": ("# ", ""),
          ".py": ("# ", ""), ".mk": ("# ", ""), "Makefile": ("# ", ""), ".md": ("<!-- ", " -->")}
LICENSE_TEXTS = {"LICENSE", "LICENSE-MIT"}
GENERATED = {"Cargo.lock"}
USAGE = f"""Keeps the license line of every file under {TREE}/ of ferrion-hardware (FerrionOS's
ADR 0047). A file that came from upstream starts with "{TAG}{INHERITED}",
a file FerrionOS added with "{TAG}{OURS}", each in the file's comment
syntax and after a "#!" line when there is one. A file that cannot hold a comment (a binary
file, a license text, {", ".join(sorted(GENERATED))}) is listed under [unmarked] of {RECORD} instead.
Run from the repository root.

Usage: {sys.argv[0]} mark <upstream tree>
       {sys.argv[0]} mark-tree <tree>
       {sys.argv[0]} check [<since> [<upstream tree>]]
       {sys.argv[0]} exempt
       {sys.argv[0]} unmarked-equals <commit>
  mark             adds the line to every file that lacks it and rewrites [unmarked]; a file is
                   upstream's when <upstream tree>, a commit or a tree, has it
  mark-tree        prints the tree that <tree> becomes when every file of it that can hold a
                   comment and lacks the line gets upstream's; nothing is checked out
  check            exit code 1 when a file has no license line or a wrong one. With <since>, a
                   commit, also when a file of <since> carries another line now, or a file added
                   after it does not say {OURS}; a file added after it that <upstream tree>
                   has must carry upstream's line instead (an upstream merge)
  exempt           prints the files listed under [unmarked]
  unmarked-equals  exit code 1 unless every file of <commit> under {TREE}/ is, without its
                   license line, byte for byte the file of <commit>"""


def git(*args, check=True, data=None, env=None):
    return subprocess.run(["git", *args], check=check, capture_output=True, input=data, env=env)


def files():
    out = git("ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", TREE).stdout
    return sorted(Path(name) for name in out.decode().split("\0") if name and Path(name).is_file())


def style(path):
    return STYLES.get(path.suffix) or STYLES.get(path.name)


def is_text(data):
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return b"\0" not in data and not data.startswith(b"\xef\xbb\xbf")


def kind(path, data):
    if path.name in LICENSE_TEXTS or path.name in GENERATED or not is_text(data):
        if style(path) is not None:
            raise SystemExit(f"{path}: has a comment syntax but is not plain UTF-8 text")
        return "unmarked"
    if style(path) is None:
        raise SystemExit(f"{path}: no comment syntax is known for this kind of file; add it to STYLES")
    return "marked"


def in_tree(tree, path):
    return git("cat-file", "-e", f"{tree}:{path.as_posix()}", check=False).returncode == 0


def position(data):
    first = data.split(b"\n", 1)[0]
    return 1 if first.startswith(b"#!") and not first.startswith(b"#![") else 0


def read_line(path, data):
    lines = data.split(b"\n")
    index = position(data)
    start, end = style(path)
    text = lines[index].rstrip(b"\r").decode() if index < len(lines) else ""
    if text.startswith(start + TAG) and text.endswith(end):
        return text[len(start + TAG):len(text) - len(end)]
    return None


def with_line(path, data, expression):
    start, end = style(path)
    lines = data.split(b"\n")
    carriage = b"\r" if lines[0].endswith(b"\r") else b""
    lines.insert(position(data), f"{start}{TAG}{expression}{end}".encode() + carriage)
    return b"\n".join(lines)


def without_line(data):
    lines = data.split(b"\n")
    del lines[position(data)]
    return b"\n".join(lines)


def unmarked():
    return tomllib.loads(Path(RECORD).read_text()).get("unmarked", {})


def mark(upstream):
    left = {}
    marked = 0
    for path in files():
        if path.as_posix() == RECORD:
            continue
        data = path.read_bytes()
        expression = INHERITED if in_tree(upstream, path) else OURS
        if kind(path, data) == "unmarked":
            left[path.relative_to(TREE).as_posix()] = "license text" if path.name in LICENSE_TEXTS else expression
        elif read_line(path, data) is None:
            path.write_bytes(with_line(path, data, expression))
            marked += 1
    record = Path(RECORD)
    text = record.read_text().split("\n[unmarked]\n")[0].rstrip("\n") + "\n"
    if not text.startswith("# " + TAG):
        text = f"# {TAG}{OURS}\n" + text
    table = "".join(f'"{name}" = "{value}"\n' for name, value in sorted(left.items()))
    record.write_text(text + "\n[unmarked]\n" + table)
    print(f"{marked} files marked, {len(left)} listed under [unmarked]")


def mark_tree(tree):
    entries = []
    for raw in git("ls-tree", "-r", "-z", tree).stdout.decode().split("\0"):
        if not raw:
            continue
        head, name = raw.split("\t", 1)
        mode, _, blob = head.split(" ")
        path = Path(name)
        data = git("cat-file", "blob", blob).stdout
        if mode != "120000" and style(path) is not None and is_text(data) and path.name not in LICENSE_TEXTS \
                and read_line(path, data) is None:
            blob = git("hash-object", "-w", "--stdin", data=with_line(path, data, INHERITED)).stdout.decode().strip()
        entries.append(f"{mode} {blob}\t{name}")
    with tempfile.TemporaryDirectory() as folder:
        env = dict(os.environ, GIT_INDEX_FILE=str(Path(folder) / "index"))
        git("update-index", "--index-info", data=("\n".join(entries) + "\n").encode(), env=env)
        print(git("write-tree", env=env).stdout.decode().strip())


def check(since, upstream):
    listed = unmarked()
    failures = []
    for path in files():
        data = path.read_bytes()
        if path.relative_to(TREE).as_posix() in listed:
            continue
        if style(path) is None or not is_text(data):
            failures.append(f"{path}: cannot hold a license line; list it under [unmarked] of {RECORD}")
            continue
        found = read_line(path, data)
        if found not in (OURS, INHERITED):
            failures.append(f"{path}: line {position(data) + 1} is not a license line ({TAG}{OURS})")
        elif since and in_tree(since, path):
            before = read_line(path, git("cat-file", "blob", f"{since}:{path.as_posix()}").stdout)
            if before is not None and before != found:
                failures.append(f"{path}: its license line was {before} at {since[:12]} and is {found} now")
        elif since:
            wanted = INHERITED if upstream and in_tree(upstream, path) else OURS
            if found != wanted:
                failures.append(f"{path}: added after {since[:12]}, so its license line is {wanted}, not {found}")
    for name in listed:
        if not (Path(TREE) / name).is_file():
            failures.append(f"{RECORD}: [unmarked] lists {name}, which is not a file")
    for failure in failures:
        print(f"FAIL {failure}")
    if not failures:
        print(f"Every file under {TREE}/ has its license line or is listed under [unmarked].")
    return 1 if failures else 0


def unmarked_equals(commit):
    names = [name for name in git("ls-tree", "-r", "-z", "--name-only", commit, "--", TREE).stdout.decode().split("\0") if name]
    differing = 0
    for name in names:
        path = Path(name)
        old = git("cat-file", "blob", f"{commit}:{name}").stdout
        new = path.read_bytes() if path.is_file() else None
        if new is not None and new != old and style(path) is not None and read_line(path, new) is not None:
            new = without_line(new)
        if new != old:
            differing += 1
            print(f"FAIL {name}: differs from {commit[:12]} by more than its license line")
    added = [path.as_posix() for path in files() if not in_tree(commit, path)]
    print(f"{len(names)} files of {commit[:12]} compared, {differing} differ; added since: {' '.join(added)}")
    return 1 if differing else 0


def main(argv):
    if len(argv) == 2 and argv[0] == "mark":
        mark(argv[1])
        return 0
    if len(argv) == 2 and argv[0] == "mark-tree":
        mark_tree(argv[1])
        return 0
    if argv[:1] == ["check"] and len(argv) <= 3:
        return check(argv[1] if len(argv) > 1 else "", argv[2] if len(argv) > 2 else "")
    if argv == ["exempt"]:
        for name in sorted(unmarked()):
            print(name)
        return 0
    if len(argv) == 2 and argv[0] == "unmarked-equals":
        return unmarked_equals(argv[1])
    print(USAGE, file=sys.stderr)
    return 0 if argv[:1] in (["-h"], ["--help"]) else 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
