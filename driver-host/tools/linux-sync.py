#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
import argparse
import hashlib
import os
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

LINUX_VERSION = "6.18.64"
LINUX_SHA256 = "020d45510c539972e6f14c5c71566878d3b7ef56fb082b3e09134f2ebaade562"
LINUX_URL = f"https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-{LINUX_VERSION}.tar.xz"
CROSS_COMPILE = "x86_64-unknown-redox-"
LICENSE_SCAN_LINES = 40
DOWNLOAD_TIMEOUT_SECONDS = 1800

HOST = Path(__file__).resolve().parent.parent
TOOLS = HOST / "tools"
CONFIG = HOST / "config"
LINUX = HOST / "linux"
GENERATED = HOST / "generated"
OBJECTS_MK = HOST / "linux-objects.mk"
MANIFEST = HOST / "linux.manifest"

PER_OBJECT_FLAG = re.compile(r"^-DKBUILD_|^-D__KBUILD_MODNAME=")
DROPPED_FLAG = re.compile(r"^-Wp,|^-fmacro-prefix-map=|^-c$|^-o$|^-I|^-include$")


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch(work):
    tarball = work / f"linux-{LINUX_VERSION}.tar.xz"
    if not tarball.exists():
        part = tarball.with_suffix(".part")
        with urllib.request.urlopen(LINUX_URL, timeout=DOWNLOAD_TIMEOUT_SECONDS) as r, open(part, "wb") as f:
            shutil.copyfileobj(r, f)
        part.rename(tarball)
    got = sha256_of(tarball)
    if got != LINUX_SHA256:
        sys.exit(f"{tarball}: sha256 {got}, pinned {LINUX_SHA256}")
    src = work / f"linux-{LINUX_VERSION}"
    if not (src / "Makefile").exists():
        with tarfile.open(tarball) as t:
            t.extractall(work, filter="tar")
    return src


def kbuild(src, out, *targets):
    env = dict(os.environ)
    env["PATH"] = f"{TOOLS}:{env['PATH']}"
    cmd = ["make", "-s", "ARCH=x86_64", f"CROSS_COMPILE={CROSS_COMPILE}", f"O={out}", f"-j{os.cpu_count()}"]
    subprocess.run(cmd + list(targets), cwd=src, env=env, check=True)


def configure(src, out):
    env = dict(os.environ)
    env["PATH"] = f"{TOOLS}:{env['PATH']}"
    subprocess.run(
        ["make", "-s", "ARCH=x86_64", f"CROSS_COMPILE={CROSS_COMPILE}", f"O={out}",
         f"KCONFIG_ALLCONFIG={CONFIG / 'ferrion.config'}", "allnoconfig"],
        cwd=src, env=env, check=True)
    kbuild(src, out, "prepare")


def object_targets():
    return [line.strip() for line in (CONFIG / "linux-objects.txt").read_text().splitlines() if line.strip()]


def cmd_files(out, targets):
    found = []
    for target in targets:
        if target.endswith("/"):
            for cmd in sorted((out / target).rglob(".*.o.cmd")):
                name = cmd.name[1:-len(".cmd")]
                if name == "built-in.a" or name.endswith(".mod.o") or (cmd.parent / name).with_suffix(".mod").exists():
                    continue
                found.append(cmd)
        else:
            path = out / target
            found.append(path.parent / f".{path.name}.cmd")
    return found


def parse_cmd(cmd, src, out):
    text = cmd.read_text()
    saved = re.search(r"^savedcmd_(\S+) := (.*)$", text, re.M)
    source = re.search(r"^source_\S+ := (\S+)$", text, re.M)
    deps = re.search(r"^deps_\S+ := \\\n(.*?)\n\n", text, re.S | re.M)
    if not (saved and source and deps):
        sys.exit(f"{cmd}: not a C object's command file")
    obj = saved.group(1)
    args = shlex.split(saved.group(2).split(" ; ")[0])[1:]
    flags, own, includes = [], [], []
    skip = False
    for i, a in enumerate(args):
        if skip:
            skip = False
            continue
        if a in ("-include", "-o"):
            skip = True
            continue
        if a.endswith(".c") or a == "-c":
            continue
        if a.startswith("-I"):
            path = a[2:]
            if path.startswith(str(src)):
                rel = os.path.relpath(path, src)
                if rel not in ("arch/x86/include", "include", "arch/x86/include/uapi", "include/uapi"):
                    own.append(f"-I$(LINUX)/{rel}")
            continue
        if PER_OBJECT_FLAG.match(a):
            own.append(shlex.quote(a))
            continue
        if DROPPED_FLAG.match(a):
            continue
        flags.append(a)
    files = []
    for line in deps.group(1).split("\n"):
        line = line.strip().rstrip("\\").strip()
        if not line or line.startswith("$(wildcard"):
            continue
        files.append(line)
    source_path = Path(source.group(1))
    return obj, source_path.relative_to(src), flags, own, files


def license_line(path):
    with open(path, errors="replace") as f:
        lines = [f.readline() for _ in range(LICENSE_SCAN_LINES)]
    for line in lines:
        m = re.search(r"SPDX-License-Identifier:\s*(.*?)\s*(\*/)?\s*$", line)
        if m:
            return m.group(1)
    for line in lines:
        if re.search(r"Permission is hereby granted|Permission to use, copy|GNU General Public License|licen[cs]e", line, re.I):
            return line.strip().strip("/*").strip()
    return "-"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, default=Path("/tmp/ferrion-hardware-linux-sync"))
    args = parser.parse_args()
    args.work.mkdir(parents=True, exist_ok=True)
    src = fetch(args.work)
    out = args.work / "kbuild"
    configure(src, out)
    targets = object_targets()
    kbuild(src, out, *targets)

    parsed = []
    copies = {}
    for cmd in cmd_files(out, targets):
        obj, source, flags, own, deps = parse_cmd(cmd, src, out)
        parsed.append((obj, source, flags, own))
        copies[str(source)] = ("linux", src / source)
        for dep in deps:
            path = Path(dep)
            if path.is_absolute():
                copies[os.path.normpath(path.relative_to(src))] = ("linux", path)
            else:
                copies[dep] = ("generated", out / dep)
    copies["include/generated/autoconf.h"] = ("generated", out / "include/generated/autoconf.h")
    shared = set.intersection(*(set(p[2]) for p in parsed))
    common = [f for f in parsed[0][2] if f in shared]
    objects = []
    for obj, source, flags, own in parsed:
        extra = [f for f in flags if f not in shared]
        codegen = [f for f in extra if not f.startswith("-W")]
        if codegen:
            sys.exit(f"{obj}: code generation flags of its own: {codegen}")
        objects.append((obj, source, own + extra))

    for tree in (LINUX, GENERATED):
        if tree.exists():
            shutil.rmtree(tree)
    rows = []
    for rel, (kind, origin) in sorted(copies.items()):
        dest = (LINUX if kind == "linux" else GENERATED) / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(origin, dest)
        rows.append(f"{kind}/{rel}\t{sha256_of(dest)}\t{license_line(dest)}")
    shutil.copyfile(out / ".config", GENERATED / "linux.config")
    rows.append(f"generated/linux.config\t{sha256_of(GENERATED / 'linux.config')}\t-")
    MANIFEST.write_text(
        f"linux-{LINUX_VERSION}.tar.xz\t{LINUX_SHA256}\tGPL-2.0 WITH Linux-syscall-note\n" + "\n".join(sorted(rows)) + "\n")

    lines = [f"LINUX_VERSION := {LINUX_VERSION}", f"LINUX_KBUILD_CFLAGS := {' '.join(common)}", ""]
    for obj, source, own in sorted(objects, key=lambda o: str(o[1])):
        stem = str(source)[:-2]
        lines.append(f"LINUX_SOURCES += {stem}")
        lines.append(f"{stem}_FLAGS := {' '.join(own)}")
    OBJECTS_MK.parent.mkdir(parents=True, exist_ok=True)
    OBJECTS_MK.write_text("\n".join(lines) + "\n")
    print(f"{len(objects)} objects, {sum(1 for r in rows if r.startswith('linux/'))} Linux files, "
          f"{sum(1 for r in rows if r.startswith('generated/'))} generated files")


if __name__ == "__main__":
    main()
