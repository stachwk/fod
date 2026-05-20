#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import array
import fcntl
import os
import time
import termios
import subprocess
import tempfile
import shutil
import unittest
import uuid
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def _run_as_nobody(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["sudo", "-n", "-u", "nobody", *args], capture_output=True, text=True, check=False)


def _nobody_access(path: Path, mode: int) -> bool:
    result = _run_as_nobody(
        sys.executable,
        "-c",
        "import os, sys; raise SystemExit(0 if os.access(sys.argv[1], int(sys.argv[2])) else 1)",
        str(path),
        str(mode),
    )
    if result.returncode not in {0, 1}:
        raise AssertionError(("nobody access check failed", str(path), mode, result.returncode, result.stderr))
    return result.returncode == 0


class FODMountSuite(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.launcher = FODMount(str(ROOT))
        cls.launcher.init_schema()

    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-suite.")
        self.mountpoint = Path(self.temp_dir.name)
        self.launcher.start(str(self.mountpoint))

    def tearDown(self):
        self.launcher.stop()
        self.temp_dir.cleanup()

    def test_files(self):
        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"files-{suffix}.bin"
        renamed = self.mountpoint / f"files-{suffix}-renamed.bin"
        expected_size = 0

        file_path.write_bytes(os.urandom(1024))
        expected_size += 1024
        listing = subprocess.check_output(["ls", "-al", str(file_path)], text=True)
        self.assertIn(file_path.name, listing)
        for block_k in (2, 5, 3, 7):
            with file_path.open("ab") as fh:
                fh.write(os.urandom(block_k * 1024))
            expected_size += block_k * 1024
            self.assertEqual(file_path.stat().st_size, expected_size)

        file_inode = file_path.stat().st_ino
        self.assertGreater(file_inode, 0)
        self.assertEqual(file_path.stat().st_nlink, 1)
        self.assertGreaterEqual(file_path.stat().st_blksize, 512)
        self.assertGreaterEqual(file_path.stat().st_blocks, 1)

        subprocess.run(["mv", str(file_path), str(renamed)], check=True)
        self.assertEqual(renamed.stat().st_ino, file_inode)
        self.assertEqual(renamed.stat().st_size, expected_size)
        subprocess.run(["rm", "-f", str(renamed)], check=True)

    def test_directories(self):
        suffix = uuid.uuid4().hex[:8]
        dir_path = self.mountpoint / f"alpha-{suffix}"
        sub_a = dir_path / "beta"
        sub_b = dir_path / "gamma"
        sub_c = self.mountpoint / f"delta-{suffix}"

        sub_a.mkdir(parents=True)
        sub_b.mkdir(parents=True)
        sub_c.mkdir(parents=True)

        self.assertEqual(dir_path.stat().st_nlink, 4)
        self.assertGreaterEqual(self.mountpoint.stat().st_nlink, 3)
        self.assertGreaterEqual(dir_path.stat().st_blocks, 1)

        beta_renamed = dir_path / "beta-renamed"
        subprocess.run(["mv", str(sub_a), str(beta_renamed)], check=True)
        time.sleep(0.2)
        self.assertTrue(beta_renamed.exists())
        self.assertFalse(sub_a.exists())

        subprocess.run(["rmdir", str(beta_renamed)], check=True)
        subprocess.run(["rmdir", str(sub_b)], check=True)
        subprocess.run(["rmdir", str(dir_path)], check=True)
        subprocess.run(["rmdir", str(sub_c)], check=True)

    def test_metadata(self):
        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"meta-{suffix}.txt"
        dir_path = self.mountpoint / f"meta-dir-{suffix}"
        dir_path.mkdir()
        file_path.write_text("metadata\n", encoding="utf-8")

        file_path.chmod(0o640)
        dir_path.chmod(0o750)

        current_uid = os.getuid()
        current_gid = os.getgid()
        os.chown(file_path, current_uid, current_gid)
        os.chown(dir_path, current_uid, current_gid)

        self.assertTrue(os.access(file_path, os.R_OK))
        self.assertTrue(os.access(file_path, os.W_OK))
        self.assertTrue(os.access(dir_path, os.X_OK))

        file_stat = file_path.stat()
        dir_stat = dir_path.stat()
        self.assertEqual(oct(file_stat.st_mode & 0o777), "0o640")
        self.assertEqual(oct(dir_stat.st_mode & 0o777), "0o750")
        self.assertEqual(file_stat.st_uid, current_uid)
        self.assertEqual(file_stat.st_gid, current_gid)
        self.assertEqual(dir_stat.st_uid, current_uid)
        self.assertEqual(dir_stat.st_gid, current_gid)
        self.assertEqual(file_stat.st_dev, dir_stat.st_dev)
        self.assertGreater(file_stat.st_dev, 0)
        self.assertGreaterEqual(file_stat.st_blocks, 1)
        self.assertGreaterEqual(dir_stat.st_blocks, 1)
        self.assertEqual(file_stat.st_size, 9)
        self.assertEqual(dir_stat.st_size, 0)
        stale_atime = file_stat.st_atime
        file_path.read_text(encoding="utf-8")
        refreshed_atime = file_path.stat().st_atime
        self.assertGreaterEqual(refreshed_atime, stale_atime)

        file_path.chmod(0o000)
        denied_read = _run_as_nobody(
            sys.executable,
            "-c",
            "from pathlib import Path; import sys; Path(sys.argv[1]).read_text(encoding='utf-8')",
            str(file_path),
        )
        self.assertNotEqual(denied_read.returncode, 0, denied_read)
        self.assertTrue(
            "PermissionError" in denied_read.stderr or "Errno 13" in denied_read.stderr,
            denied_read.stderr,
        )

    def test_access_modes(self):
        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"access-{suffix}.txt"
        dir_path = self.mountpoint / f"access-dir-{suffix}"
        try:
            file_path.write_text("access\n", encoding="utf-8")
            dir_path.mkdir()

            file_path.chmod(0o640)
            dir_path.chmod(0o750)

            self.assertTrue(os.access(file_path, os.R_OK))
            self.assertTrue(os.access(file_path, os.W_OK))
            self.assertFalse(os.access(file_path, os.X_OK))
            self.assertTrue(os.access(dir_path, os.R_OK))
            self.assertTrue(os.access(dir_path, os.W_OK))
            self.assertTrue(os.access(dir_path, os.X_OK))

            file_path.chmod(0o000)
            dir_path.chmod(0o000)
            self.assertFalse(_nobody_access(file_path, os.R_OK))
            self.assertFalse(_nobody_access(file_path, os.W_OK))
            self.assertFalse(_nobody_access(file_path, os.X_OK))
            self.assertFalse(_nobody_access(dir_path, os.R_OK))
            self.assertFalse(_nobody_access(dir_path, os.W_OK))
            self.assertFalse(_nobody_access(dir_path, os.X_OK))
        finally:
            try:
                file_path.chmod(0o640)
            except Exception:
                pass
            try:
                dir_path.chmod(0o750)
            except Exception:
                pass
            try:
                file_path.unlink()
            except Exception:
                pass
            try:
                dir_path.rmdir()
            except Exception:
                pass

    def test_ioctl_fionread(self):
        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"ioctl-{suffix}.txt"
        payload = b"ioctl payload\n"
        file_path.write_bytes(payload)

        fd = os.open(file_path, os.O_RDONLY)
        try:
            buf = array.array("i", [0])
            fcntl.ioctl(fd, termios.FIONREAD, buf, True)
            self.assertEqual(buf[0], len(payload))
        finally:
            os.close(fd)

    def test_runtime_features_off(self):
        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"runtime-off-{suffix}.txt"
        file_path.write_text("runtime-off\n", encoding="utf-8")

        acl_blob = b"\x02\x00\x00\x00"
        for name, value in (
            ("security.selinux", b"system_u:object_r:tmp_t:s0"),
            ("system.posix_acl_access", acl_blob),
        ):
            with self.assertRaises(OSError) as ctx:
                os.setxattr(file_path, name, value)
            self.assertIn(ctx.exception.errno, {errno.EOPNOTSUPP, errno.ENOTSUP, errno.EPERM, errno.ENODATA})

        self.assertNotIn("security.selinux", os.listxattr(file_path))
        self.assertNotIn("system.posix_acl_access", os.listxattr(file_path))

    def test_posix_fuse_contract(self):
        suffix = uuid.uuid4().hex[:8]
        contract_dir = self.mountpoint / f"contract-dir-{suffix}"
        contract_file = contract_dir / "payload.txt"
        renamed_file = self.mountpoint / f"contract-renamed-{suffix}.txt"
        contract_link = self.mountpoint / f"contract-link-{suffix}"
        payload = "contract payload\n"

        root_stat = self.mountpoint.stat()
        self.assertGreater(root_stat.st_dev, 0)
        self.assertGreater(root_stat.st_ino, 0)

        try:
            contract_dir.mkdir()
            contract_file.write_text(payload, encoding="utf-8")
            contract_file.chmod(0o644)
            contract_dir.chmod(0o755)

            file_stat = contract_file.stat()
            dir_stat = contract_dir.stat()
            self.assertGreater(file_stat.st_ino, 0)
            self.assertEqual(file_stat.st_dev, root_stat.st_dev)
            self.assertEqual(dir_stat.st_dev, root_stat.st_dev)
            self.assertEqual(file_stat.st_nlink, 1)
            self.assertTrue(_nobody_access(contract_file, os.R_OK))
            self.assertFalse(_nobody_access(contract_file, os.W_OK))
            self.assertFalse(_nobody_access(contract_file, os.X_OK))
            self.assertTrue(_nobody_access(contract_dir, os.R_OK))
            self.assertFalse(_nobody_access(contract_dir, os.W_OK))
            self.assertTrue(_nobody_access(contract_dir, os.X_OK))

            os.rename(contract_file, renamed_file)
            self.assertEqual(renamed_file.stat().st_ino, file_stat.st_ino)
            self.assertEqual(renamed_file.read_text(encoding="utf-8"), payload)

            os.symlink(renamed_file.name, contract_link)
            self.assertTrue(contract_link.is_symlink())
            self.assertEqual(os.readlink(contract_link), renamed_file.name)
            self.assertEqual(contract_link.read_text(encoding="utf-8"), payload)
        finally:
            try:
                contract_link.unlink(missing_ok=True)
            except Exception:
                pass
            try:
                renamed_file.unlink(missing_ok=True)
            except Exception:
                pass
            try:
                contract_file.unlink(missing_ok=True)
            except Exception:
                pass
            try:
                contract_dir.rmdir()
            except Exception:
                pass

    def test_selinux_runtime_feature_on(self):
        if self.launcher.selinux not in {"on", "auto"}:
            self.skipTest("SELinux runtime feature is disabled for this mount")

        suffix = uuid.uuid4().hex[:8]
        file_path = self.mountpoint / f"selinux-on-{suffix}.txt"
        file_path.write_text("selinux-on\n", encoding="utf-8")

        selinux_value = b"system_u:object_r:tmp_t:s0"
        try:
            os.setxattr(file_path, "security.selinux", selinux_value)
        except OSError as exc:
            if exc.errno in {errno.EPERM, errno.EOPNOTSUPP, errno.ENOTSUP}:
                self.skipTest("SELinux xattr is not enabled on this host")
            raise
        self.assertEqual(os.getxattr(file_path, "security.selinux"), selinux_value)
        self.assertIn("security.selinux", os.listxattr(file_path))

    def test_symlink(self):
        suffix = uuid.uuid4().hex[:8]
        payload = self.mountpoint / f"payload-{suffix}.txt"
        link_path = self.mountpoint / f"payload-link-{suffix}"
        renamed = self.mountpoint / f"payload-link-renamed-{suffix}"
        orphaned = self.mountpoint / f"payload-orphaned-{suffix}"
        payload_value = "symlink smoke payload\n"
        payload.write_text(payload_value, encoding="utf-8")

        subprocess.run(["ln", "-s", str(payload), str(link_path)], check=True)
        self.assertTrue(link_path.is_symlink())
        self.assertEqual(os.readlink(link_path), str(payload))
        self.assertEqual(link_path.read_text(encoding="utf-8"), payload_value)

        subprocess.run(["mv", str(link_path), str(renamed)], check=True)
        self.assertTrue(renamed.is_symlink())
        self.assertEqual(os.readlink(renamed), str(payload))
        self.assertEqual(renamed.read_text(encoding="utf-8"), payload_value)

        subprocess.run(["ln", "-s", str(payload), str(orphaned)], check=True)
        subprocess.run(["rm", "-f", str(payload)], check=True)
        self.assertTrue(orphaned.is_symlink())
        self.assertFalse(orphaned.exists())
        self.assertEqual(os.readlink(orphaned), str(payload))
        ls_output = subprocess.check_output(["ls", "-al", str(orphaned)], text=True)
        self.assertIn(f"{orphaned} -> {payload}", ls_output)

        subprocess.run(["rm", "-f", str(renamed)], check=True)
        subprocess.run(["rm", "-f", str(orphaned)], check=True)

    def test_df(self):
        ph = subprocess.check_output(["df", "-Ph", str(self.mountpoint)], text=True)
        phi = subprocess.check_output(["df", "-Phi", str(self.mountpoint)], text=True)
        self.assertIn(str(self.mountpoint), ph)
        self.assertIn(str(self.mountpoint), phi)

    def test_replica_read_only(self):
        replica_launcher = FODMount(str(ROOT), role="replica")
        replica_mountpoint = Path(tempfile.mkdtemp(prefix="fod-replica-suite.", dir="/tmp"))
        seed_dir: Path | None = None
        seed_file: Path | None = None
        try:
            suffix = uuid.uuid4().hex[:8]
            seed_dir = self.mountpoint / f"replica_seed_{suffix}"
            seed_file = seed_dir / "seed.txt"

            seed_dir.mkdir(mode=0o755)
            seed_file.write_text("seed-data", encoding="utf-8")

            replica_launcher.start(str(replica_mountpoint))
            replica_seed_file = replica_mountpoint / seed_dir.name / "seed.txt"

            replica_stat = None
            for _ in range(30):
                try:
                    if replica_seed_file.exists():
                        replica_stat = replica_seed_file.stat()
                        if replica_stat.st_size == len("seed-data"):
                            break
                except OSError as exc:
                    if exc.errno != errno.EROFS:
                        raise
                time.sleep(0.1)

            self.assertIsNotNone(replica_stat)
            self.assertEqual(replica_stat.st_size, len("seed-data"))

            try:
                replica_output = replica_seed_file.read_text(encoding="utf-8")
                self.assertEqual(replica_output, "seed-data")
            except OSError as exc:
                if exc.errno != errno.EROFS:
                    raise

            with self.assertRaises(OSError) as touch_exc:
                (replica_mountpoint / "new.txt").write_text("x", encoding="utf-8")
            self.assertIn(touch_exc.exception.errno, {errno.EROFS, errno.EPERM})

            with self.assertRaises(OSError) as mkdir_exc:
                (replica_mountpoint / "newdir").mkdir()
            self.assertIn(mkdir_exc.exception.errno, {errno.EROFS, errno.EPERM})
        finally:
            replica_launcher.stop()
            shutil.rmtree(replica_mountpoint, ignore_errors=True)

            if seed_file is not None:
                try:
                    seed_file.unlink(missing_ok=True)
                except Exception:
                    pass

            if seed_dir is not None:
                try:
                    seed_dir.rmdir()
                except Exception:
                    pass


if __name__ == "__main__":
    unittest.main(verbosity=2)
