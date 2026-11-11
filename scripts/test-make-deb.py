#!/usr/bin/env python3
"""Tests for make-deb.py. Run: python3 -I scripts/test-make-deb.py"""
import hashlib
import importlib.util
import io
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("make_deb", os.path.join(here, "make-deb.py"))
make_deb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(make_deb)


class MakeDebTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.root = os.path.join(t, "root")
        files = (
            ("usr/bin/koyomi", b"bin"),
            ("usr/share/doc/koyomi/README.md", b"readme\n"),
            ("usr/share/koyomi/install.sh", b"#!/bin/sh\n"),
        )
        for rel, content in files:
            p = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(content)
        self.ctl_dir = os.path.join(t, "ctl")
        os.makedirs(self.ctl_dir)
        with open(os.path.join(self.ctl_dir, "control"), "wb") as f:
            f.write(b"Package: koyomi\n")
        with open(os.path.join(self.ctl_dir, "postinst"), "wb") as f:
            f.write(b"#!/bin/sh\nexit 0\n")

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, name):
        out = os.path.join(self.tmp.name, name)
        make_deb.build(self.root, os.path.join(self.ctl_dir, "control"), out, set(), self.ctl_dir, 1700000000)
        with open(out, "rb") as f:
            return f.read()

    def tars(self, blob):
        members = make_deb.read_ar(blob)
        tars = {}
        for n, _, d in members:
            if n.endswith(".tar.gz"):
                tars[n] = tarfile.open(fileobj=io.BytesIO(d), mode="r:gz")
        return members, tars

    def test_ar_layout(self):
        blob = self.build("a.deb")
        self.assertTrue(blob.startswith(b"!<arch>\n"))
        members, _ = self.tars(blob)
        self.assertEqual([m[0] for m in members], ["debian-binary", "control.tar.gz", "data.tar.gz"])
        self.assertEqual(members[0][2], b"2.0\n")
        for _, hdr, _ in members:
            self.assertEqual(hdr[28:34].strip(), b"0")
            self.assertEqual(hdr[34:40].strip(), b"0")
            self.assertEqual(hdr[40:48].strip(), b"100644")

    def test_modes_and_owners(self):
        _, tars = self.tars(self.build("a.deb"))
        # tarfile strips the trailing slash of a directory name when reading.
        data = {ti.name: ti for ti in tars["data.tar.gz"]}
        for d in (".", "./usr", "./usr/bin", "./usr/share/doc/koyomi"):
            self.assertEqual(data[d].mode, 0o755, d)
            self.assertTrue(data[d].isdir())
        self.assertEqual(data["./usr/bin/koyomi"].mode, 0o755)
        self.assertEqual(data["./usr/share/koyomi/install.sh"].mode, 0o755)
        self.assertEqual(data["./usr/share/doc/koyomi/README.md"].mode, 0o644)
        for ti in list(data.values()) + list(tars["control.tar.gz"]):
            self.assertEqual((ti.uid, ti.gid, ti.uname, ti.gname), (0, 0, "root", "root"))
        ctl = {ti.name: ti for ti in tars["control.tar.gz"]}
        self.assertEqual(ctl["./postinst"].mode, 0o755)
        self.assertEqual(ctl["./control"].mode, 0o644)
        self.assertNotIn("./copyright", ctl)

    def test_md5sums(self):
        _, tars = self.tars(self.build("a.deb"))
        lines = tars["control.tar.gz"].extractfile("./md5sums").read().decode().splitlines()
        self.assertEqual(lines, sorted(lines, key=lambda x: x.split("  ", 1)[1]))
        seen = {}
        for line in lines:
            h, p = line.split("  ", 1)
            self.assertFalse(p.startswith(("./", "/")))
            seen[p] = h
        reg = [ti for ti in tars["data.tar.gz"] if ti.isreg()]
        self.assertEqual(sorted(seen), sorted(ti.name[2:] for ti in reg))
        for ti in reg:
            content = tars["data.tar.gz"].extractfile(ti).read()
            self.assertEqual(seen[ti.name[2:]], hashlib.md5(content).hexdigest())

    def test_reproducible(self):
        self.assertEqual(self.build("a.deb"), self.build("b.deb"))

    def test_gzip_header_has_no_name(self):
        members = make_deb.read_ar(self.build("a.deb"))
        for n, _, d in members[1:]:
            self.assertEqual(d[3], 0, n)  # FLG: no FNAME

    def test_name_too_long_for_ustar_is_refused(self):
        # One path part over 100 bytes cannot be split into ustar's prefix and name.
        with self.assertRaises(SystemExit):
            make_deb.tar_gz([("./usr/share/" + "z" * 101, 0o644, b"")], 0)

    def test_bad_mtime_is_refused(self):
        for v in ("soon", "-1"):
            with self.assertRaises(SystemExit):
                make_deb.epoch(v, "--mtime")


class ShellSyntaxTest(unittest.TestCase):
    """The packaging shell scripts parse (`sh -n` / `bash -n`; bashisms are not caught here)."""

    def test_scripts_parse(self):
        repo = os.path.dirname(here)
        scripts = (
            ("sh", "packaging/deb/preinst"),
            ("sh", "packaging/deb/postinst"),
            ("sh", "packaging/deb/prerm"),
            ("sh", "packaging/deb/postrm"),
            ("sh", "packaging/linux/koyomi-user"),
            ("bash", "packaging/linux/koyomi-secret"),
            ("bash", "packaging/linux/install.sh"),
            ("bash", "scripts/package-linux.sh"),
        )
        for shell, rel in scripts:
            exe = shutil.which(shell)
            if exe is None:
                self.skipTest(shell + " not found")
            r = subprocess.run([exe, "-n", os.path.join(repo, rel)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, rel + ": " + r.stderr)


if __name__ == "__main__":
    unittest.main()
