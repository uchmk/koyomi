#!/usr/bin/env python3
"""Write a .deb without dpkg-deb or ar (stdlib only; run as `python3 -I`).

  make-deb.py [--exec <path>]... [--scripts <dir>] [--mtime <epoch>] <root-folder> <control-file> <out.deb>
  make-deb.py --list <deb>

The .deb is an ar archive of debian-binary, control.tar.gz and data.tar.gz.
Everything is owned by 0:0 and the times come from SOURCE_DATE_EPOCH (else
--mtime), so two runs give the same bytes.
"""
import gzip
import hashlib
import io
import os
import sys
import tarfile

MAINTAINER_SCRIPTS = ("preinst", "postinst", "prerm", "postrm")
# Paths (relative to the root folder) that are executable besides --exec.
EXEC_TABLE = ("usr/bin/", "usr/sbin/", "usr/share/koyomi/install.sh")


def die(msg):
    print("make-deb: " + msg, file=sys.stderr)
    sys.exit(1)


def epoch(text, what):
    try:
        n = int(text)
    except ValueError:
        die("%s is not a whole number of seconds: %r" % (what, text))
    if n < 0:
        die("%s is before 1970: %d" % (what, n))
    return n


def is_exec(rel, extra):
    if rel in extra:
        return True
    return any(rel == t or (t.endswith("/") and rel.startswith(t)) for t in EXEC_TABLE)


def tar_gz(entries, mtime):
    """entries: list of (name, mode, data or None for a directory). Returns gzip bytes."""
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as tf:
        for name, mode, data in sorted(entries, key=lambda e: e[0]):
            ti = tarfile.TarInfo(name)
            ti.mtime = mtime
            ti.uid = ti.gid = 0
            ti.uname = ti.gname = "root"
            ti.mode = mode
            try:
                if data is None:
                    ti.type = tarfile.DIRTYPE
                    tf.addfile(ti)
                else:
                    ti.size = len(data)
                    tf.addfile(ti, io.BytesIO(data))
            except ValueError as e:
                die("path does not fit in a ustar header: %s (%s)" % (name, e))
    out = io.BytesIO()
    with gzip.GzipFile(filename="", fileobj=out, mode="wb", mtime=mtime, compresslevel=9) as gz:
        gz.write(raw.getvalue())
    return out.getvalue()


def collect_data(root, extra_exec):
    """Walk the root folder: a directory entry for every parent, then the files."""
    entries = {"./": (0o755, None)}
    files = []  # (rel, bytes)
    for dirpath, dirnames, filenames in os.walk(root):
        for n in dirnames + filenames:
            full = os.path.join(dirpath, n)
            if os.path.islink(full):
                die("symbolic link not supported: " + full)
            rel = os.path.relpath(full, root).replace(os.sep, "/")
            if os.path.isdir(full):
                entries["./" + rel + "/"] = (0o755, None)
            else:
                with open(full, "rb") as f:
                    data = f.read()
                files.append((rel, data))
                entries["./" + rel] = (0o755 if is_exec(rel, extra_exec) else 0o644, data)
    return [(n, m, d) for n, (m, d) in entries.items()], files


def ar_member(name, data, mtime):
    hdr = "%-16s%-12d%-6d%-6d%-8s%-10d`\n" % (name, mtime, 0, 0, "100644", len(data))
    assert len(hdr) == 60
    return hdr.encode("ascii") + data + (b"\n" if len(data) % 2 else b"")


def build(root, control, out, extra_exec, scripts_dir, mtime):
    if not os.path.isdir(root):
        die("root folder not found: " + root)
    data_entries, files = collect_data(root, extra_exec)
    md5 = sorted((rel, hashlib.md5(d).hexdigest()) for rel, d in files)
    md5sums = "".join("%s  %s\n" % (h, rel) for rel, h in md5).encode("utf-8")
    with open(control, "rb") as f:
        control_bytes = f.read()
    ctl = [("./", 0o755, None), ("./control", 0o644, control_bytes), ("./md5sums", 0o644, md5sums)]
    for s in MAINTAINER_SCRIPTS:
        p = os.path.join(scripts_dir, s)
        if os.path.isfile(p):
            with open(p, "rb") as f:
                ctl.append(("./" + s, 0o755, f.read()))
    blob = b"!<arch>\n"
    blob += ar_member("debian-binary", b"2.0\n", mtime)
    blob += ar_member("control.tar.gz", tar_gz(ctl, mtime), mtime)
    blob += ar_member("data.tar.gz", tar_gz(data_entries, mtime), mtime)
    with open(out, "wb") as f:
        f.write(blob)


def read_ar(blob):
    if blob[:8] != b"!<arch>\n":
        die("not an ar archive")
    pos, members = 8, []
    while pos < len(blob):
        hdr = blob[pos:pos + 60]
        if len(hdr) < 60 or hdr[58:60] != b"`\n":
            die("bad ar header at %d" % pos)
        try:
            size = int(hdr[48:58])
        except ValueError:
            die("bad ar member size at %d" % pos)
        members.append((hdr[:16].decode().strip(), hdr, blob[pos + 60:pos + 60 + size]))
        pos += 60 + size + (size % 2)
    return members


def list_deb(path):
    with open(path, "rb") as f:
        members = read_ar(f.read())
    for name, hdr, data in members:
        print("%s  %d bytes  mtime=%s uid=%s gid=%s mode=%s" % (
            name, len(data), hdr[16:28].decode().strip(), hdr[28:34].decode().strip(),
            hdr[34:40].decode().strip(), hdr[40:48].decode().strip()))
        if name.endswith(".tar.gz"):
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tf:
                for ti in tf:
                    print("  %s %o %s/%s %d %s" % (
                        "d" if ti.isdir() else "-", ti.mode, ti.uname, ti.gname, ti.size,
                        ti.name + "/" if ti.isdir() else ti.name))
        elif name == "debian-binary":
            print("  " + data.decode().strip())


def main(argv):
    if len(argv) == 2 and argv[0] == "--list":
        list_deb(argv[1])
        return
    extra, scripts_dir, mtime, pos = set(), None, None, []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--exec", "--scripts", "--mtime"):
            if i + 1 >= len(argv):
                die(a + " needs a value")
            v = argv[i + 1]
            if a == "--exec":
                extra.add(v.lstrip("/"))
            elif a == "--scripts":
                scripts_dir = v
            else:
                mtime = epoch(v, "--mtime")
            i += 2
        else:
            pos.append(a)
            i += 1
    if len(pos) != 3:
        die("usage: make-deb.py [--exec p]... [--scripts dir] [--mtime n] <root> <control> <out.deb> | --list <deb>")
    if os.environ.get("SOURCE_DATE_EPOCH"):
        mtime = epoch(os.environ["SOURCE_DATE_EPOCH"], "SOURCE_DATE_EPOCH")
    if mtime is None:
        die("no SOURCE_DATE_EPOCH and no --mtime")
    root, control, out = pos
    build(root, control, out, extra, scripts_dir or os.path.dirname(os.path.abspath(control)), mtime)


if __name__ == "__main__":
    main(sys.argv[1:])
