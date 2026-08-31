#!/usr/bin/env python
# -*- coding: utf-8 -*-

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def write_executable(path, content):
    path.write_text(content, encoding='utf-8')
    path.chmod(0o755)

class MakefileUninstallOnRootTests(unittest.TestCase):
    def run_make(self, mode):
        with tempfile.TemporaryDirectory(prefix='fod-uninstall-test-') as tmp_dir:
            tmp = Path(tmp_dir)
            log = tmp / 'sudo.log'
            state = tmp / 'findmnt.state'
            fake_bin = tmp / 'bin'
            fake_bin.mkdir()

            if mode == 'none':
                findmnt = '#!/bin/sh\nexit 0\n'
            elif mode == 'remain':
                findmnt = "#!/bin/sh\nprintf '%s\\n' '/mnt/fod-a fuse.fod'\nprintf '%s\\n' '/mnt/fod-b fuse'\n"
            else:
                findmnt = ('#!/bin/sh\n' + f'state={state}\n' + 'if [ ! -e "$state" ]; then\n' + '  : >"$state"\n' + "  printf '%s\\n' '/mnt/fod-a fuse.fod'\n" + "  printf '%s\\n' '/mnt/fod-b fuse'\n" + 'fi\n')
            write_executable(fake_bin / 'findmnt', findmnt)

            fail = 'if [ "$1" = "umount" ]; then exit 23; fi\n' if mode == 'fail' else ''
            sudo = '#!/bin/sh\n' + f"printf '%s\\n' \"$*\" >> {log}\n" + fail + 'exit 0\n'
            write_executable(fake_bin / 'sudo', sudo)

            env = os.environ.copy()
            env['PATH'] = str(fake_bin) + os.pathsep + env.get('PATH', '')
            proc = subprocess.run([
                'make', '--no-print-directory', 'uninstall-root',
                f'SUDO={fake_bin / "sudo"}',
                f'FOD_CONFIG_DEST={tmp / "etc/fod/fod_config.ini"}',
                f'MOUNT_HELPER_DEST={tmp / "usr/local/sbin/mount.fod"}',
            ], cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            entries = log.read_text(encoding='utf-8').splitlines() if log.exists() else []
            return proc, entries

    def test_unmounts_before_remove(self):
        proc, entries = self.run_make('ok')
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(entries[0], 'umount -- /mnt/fod-a')
        self.assertEqual(entries[1], 'umount -- /mnt/fod-b')
        rm_index = next(i for i, e in enumerate(entries) if e.startswith('rm -f '))
        self.assertGreater(rm_index, 1)

    def test_umount_failure_aborts_before_remove(self):
        proc, entries = self.run_make('fail')
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(any(e.startswith('rm -f ') for e in entries))

    def test_remaining_mount_aborts_before_remove(self):
        proc, entries = self.run_make('remain')
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(any(e.startswith('rm -f ') for e in entries))

    def test_no_mounts_still_removes_installation(self):
        proc, entries = self.run_make('none')
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertFalse(any(e.startswith('umount -- ') for e in entries))
        self.assertTrue(any(e.startswith('rm -f ') for e in entries))

if __name__ == '__main__':
    unittest.main()
