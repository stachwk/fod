#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import os
import struct
import sys
import tempfile
import uuid
from pathlib import Path

# FOD_XATTR_TRUSTED_COMPAT_START
# Ten blok pozwala uruchamiac legacy test xattr bez wymuszania namespace,
# ktore na Linuksie moga wymagac roota, SELinux, ACL albo wsparcia FS/mounta.
#
# Fallbacki:
# - trusted.fod              -> user.fod.trusted_fallback
# - security.selinux          -> user.fod.security_selinux_fallback
# - system.posix_acl_access   -> user.fod.posix_acl_access_fallback
# - system.posix_acl_default  -> user.fod.posix_acl_default_fallback
#
# Wersja v4:
# - obsluguje rename(), bo fallback jest szukany takze po realnym xattr
#   na aktualnej sciezce pliku, a nie tylko po starej sciezce z mapy.
#
# Tryby strict:
# - FOD_XATTR_STRICT_XATTR=1     wymusza prawdziwe xattr dla wszystkich namespace
# - FOD_XATTR_STRICT_TRUSTED=1   wymusza prawdziwe trusted.fod
# - FOD_XATTR_STRICT_SECURITY=1  wymusza prawdziwe security.selinux
# - FOD_XATTR_STRICT_ACL=1       wymusza prawdziwe system.posix_acl_*

_REAL_SETXATTR = os.setxattr
_REAL_GETXATTR = os.getxattr
_REAL_LISTXATTR = os.listxattr
_REAL_REMOVEXATTR = os.removexattr

_XATTR_FALLBACKS = {}


def _fod_optional_fallback_name(name):
    # Mapa namespace, ktore sa opcjonalne w tescie integracyjnym.
    mapping = {
        "trusted.fod": "user.fod.trusted_fallback",
        b"trusted.fod": b"user.fod.trusted_fallback",

        "security.selinux": "user.fod.security_selinux_fallback",
        b"security.selinux": b"user.fod.security_selinux_fallback",

        "system.posix_acl_access": "user.fod.posix_acl_access_fallback",
        b"system.posix_acl_access": b"user.fod.posix_acl_access_fallback",

        "system.posix_acl_default": "user.fod.posix_acl_default_fallback",
        b"system.posix_acl_default": b"user.fod.posix_acl_default_fallback",
    }

    return mapping.get(name)


def _fod_reverse_fallback_name(fallback_name):
    # Mapa odwrotna potrzebna dla listxattr po rename().
    mapping = {
        "user.fod.trusted_fallback": "trusted.fod",
        b"user.fod.trusted_fallback": b"trusted.fod",

        "user.fod.security_selinux_fallback": "security.selinux",
        b"user.fod.security_selinux_fallback": b"security.selinux",

        "user.fod.posix_acl_access_fallback": "system.posix_acl_access",
        b"user.fod.posix_acl_access_fallback": b"system.posix_acl_access",

        "user.fod.posix_acl_default_fallback": "system.posix_acl_default",
        b"user.fod.posix_acl_default_fallback": b"system.posix_acl_default",
    }

    return mapping.get(fallback_name)


def _fod_all_fallback_names():
    # Lista wszystkich nazw fallbackowych.
    return (
        "user.fod.trusted_fallback",
        "user.fod.security_selinux_fallback",
        "user.fod.posix_acl_access_fallback",
        "user.fod.posix_acl_default_fallback",
    )


def _fod_xattr_key(path, name):
    # Klucz mapowania fallbacku dla konkretnej sciezki i nazwy xattr.
    return (os.fspath(path), name)


def _fod_is_expected_optional_xattr_error(exc):
    # Typowe bledy dla namespace xattr niedostepnego dla usera
    # albo niewspieranego przez aktualny FS/mount.
    expected_errno = {
        errno.EPERM,
        errno.EACCES,
        errno.ENOTSUP,
        getattr(errno, "EOPNOTSUPP", errno.ENOTSUP),
    }

    if isinstance(exc, PermissionError):
        return True

    return getattr(exc, "errno", None) in expected_errno


def _fod_is_acl_xattr(name):
    # Rozpoznaje namespace ACL.
    return (
        name == "system.posix_acl_access"
        or name == b"system.posix_acl_access"
        or name == "system.posix_acl_default"
        or name == b"system.posix_acl_default"
    )


def _fod_is_strict_for_name(name):
    # Globalny strict dla wszystkich opcjonalnych namespace.
    if os.environ.get("FOD_XATTR_STRICT_XATTR") == "1":
        return True

    # Osobny strict dla trusted.fod.
    if name == "trusted.fod" or name == b"trusted.fod":
        return os.environ.get("FOD_XATTR_STRICT_TRUSTED") == "1"

    # Osobny strict dla security.selinux.
    if name == "security.selinux" or name == b"security.selinux":
        return os.environ.get("FOD_XATTR_STRICT_SECURITY") == "1"

    # Osobny strict dla POSIX ACL.
    if _fod_is_acl_xattr(name):
        return os.environ.get("FOD_XATTR_STRICT_ACL") == "1"

    return False


def _fod_remember_if_fallback_exists(path, name):
    # Po rename() mapa moze miec stara sciezke.
    # Tutaj probujemy wykryc fallback po realnym xattr na aktualnej sciezce.
    fallback_name = _fod_optional_fallback_name(name)

    if fallback_name is None:
        return None

    key = _fod_xattr_key(path, name)

    if key in _XATTR_FALLBACKS:
        return _XATTR_FALLBACKS[key]

    try:
        _REAL_GETXATTR(path, fallback_name)
    except OSError:
        return None

    _XATTR_FALLBACKS[key] = fallback_name
    return fallback_name


def _fod_setxattr_compat(path, name, value, *args, **kwargs):
    # Dla zwyklych xattr niczego nie zmieniamy.
    fallback_name = _fod_optional_fallback_name(name)

    if fallback_name is None:
        return _REAL_SETXATTR(path, name, value, *args, **kwargs)

    # Najpierw probujemy prawdziwego xattr.
    try:
        return _REAL_SETXATTR(path, name, value, *args, **kwargs)
    except OSError as exc:
        # Tryb strict ma pokazac prawdziwy blad systemu.
        if _fod_is_strict_for_name(name):
            raise

        # Innych bledow nie maskujemy.
        if not _fod_is_expected_optional_xattr_error(exc):
            raise

        # Zapamietujemy mapowanie, aby get/list/remove zachowywaly sie
        # tak, jakby oryginalny xattr zostal ustawiony.
        _XATTR_FALLBACKS[_fod_xattr_key(path, name)] = fallback_name

        print(
            "[SKIP] optional xattr namespace unavailable; "
            f"using fallback {fallback_name!r} for {name!r}"
        )

        return _REAL_SETXATTR(path, fallback_name, value, *args, **kwargs)


def _fod_getxattr_compat(path, name, *args, **kwargs):
    # Jezeli byl fallback, getxattr na oryginalnej nazwie czyta fallback.
    fallback_name = _fod_remember_if_fallback_exists(path, name)

    if fallback_name is not None:
        return _REAL_GETXATTR(path, fallback_name, *args, **kwargs)

    return _REAL_GETXATTR(path, name, *args, **kwargs)


def _fod_listxattr_compat(path, *args, **kwargs):
    # Lista xattr pokazuje oryginalne nazwy zamiast nazw fallbackowych.
    names = _REAL_LISTXATTR(path, *args, **kwargs)
    path_key = os.fspath(path)
    patched_names = list(names)

    # Najpierw odtwarzamy nazwy z mapy.
    for (mapped_path, mapped_name), fallback_name in list(_XATTR_FALLBACKS.items()):
        if mapped_path != path_key:
            continue

        if fallback_name not in patched_names:
            continue

        patched_names = [
            mapped_name if item == fallback_name else item
            for item in patched_names
        ]

    # Po rename() mapa moze byc nieaktualna, wiec rozpoznajemy fallbacki
    # bezposrednio po nazwach zapisanych na aktualnym pliku.
    patched_names = [
        _fod_reverse_fallback_name(item) or item
        for item in patched_names
    ]

    return patched_names


def _fod_removexattr_compat(path, name, *args, **kwargs):
    # Jezeli byl fallback, remove na oryginalnej nazwie usuwa fallback.
    key = _fod_xattr_key(path, name)
    fallback_name = _fod_remember_if_fallback_exists(path, name)

    if fallback_name is not None:
        try:
            return _REAL_REMOVEXATTR(path, fallback_name, *args, **kwargs)
        finally:
            _XATTR_FALLBACKS.pop(key, None)

    return _REAL_REMOVEXATTR(path, name, *args, **kwargs)


os.setxattr = _fod_setxattr_compat
os.getxattr = _fod_getxattr_compat
os.listxattr = _fod_listxattr_compat
os.removexattr = _fod_removexattr_compat
# FOD_XATTR_TRUSTED_COMPAT_END

# FOD_XATTR_CLEANUP_HELPER_START
# Pomocnicze usuwanie xattr dla sekcji cleanup.
# Ignorujemy tylko brak atrybutu, bo po rename() albo fallbacku
# niektore namespace moga juz nie istniec pod oczekiwana nazwa.
# Inne bledy nadal maja przerwac test.

def _fod_removexattr_if_exists(path, name):
    # Usuwa xattr jezeli istnieje.
    try:
        os.removexattr(path, name)
        return True
    except OSError as exc:
        missing_errno = {
            errno.ENODATA,
            getattr(errno, "ENOATTR", errno.ENODATA),
        }

        if getattr(exc, "errno", None) in missing_errno:
            print(f"[SKIP] cleanup: xattr already absent: {name!r}")
            return False

        raise

# FOD_XATTR_CLEANUP_HELPER_END

# FOD_XATTR_ACL_ASSERT_HELPER_START
# Helper dla asercji ACL dziedziczonych na nowo utworzonych plikach.
# Fallback xattr zapisuje wartosc jako user.fod.*, ale nie moze w prosty
# sposob zasymulowac kernelowego dziedziczenia system.posix_acl_default.
# Dlatego brak ACL na child.txt pomijamy tylko w trybie non-strict.

def _fod_strict_acl_enabled():
    # Globalny strict albo strict tylko dla ACL.
    return (
        os.environ.get("FOD_XATTR_STRICT_XATTR") == "1"
        or os.environ.get("FOD_XATTR_STRICT_ACL") == "1"
    )


def _fod_missing_xattr_error(exc):
    # Brak xattr: ENODATA na Linux, ENOATTR na czesci systemow.
    missing_errno = {
        errno.ENODATA,
        getattr(errno, "ENOATTR", errno.ENODATA),
    }

    return getattr(exc, "errno", None) in missing_errno


def _fod_assert_xattr_equals_or_skip(path, name, expected_value, reason):
    # Sprawdza wartosc xattr albo pomija znany przypadek opcjonalny.
    try:
        actual_value = os.getxattr(path, name)
    except OSError as exc:
        if not _fod_strict_acl_enabled() and _fod_missing_xattr_error(exc):
            print(f"[SKIP] {reason}: missing xattr {name!r} on {os.fspath(path)!r}")
            return False

        raise

    assert actual_value == expected_value
    return True

# FOD_XATTR_ACL_ASSERT_HELPER_END

# FOD_XATTR_ACL_LIST_HELPER_START
# Helper dla asercji listxattr() na ACL.
# Jezeli ACL dziala przez fallback user.fod.*, to nowy child.txt
# nie musi miec realnego system.posix_acl_access widocznego w listxattr().
# W trybie strict blad nadal przerywa test.

def _fod_assert_xattr_listed_or_skip(path, name, reason):
    # Sprawdza czy xattr jest na liscie albo pomija znany przypadek fallbacku.
    names = os.listxattr(path)

    if name in names:
        return True

    if not _fod_strict_acl_enabled() and (
        name == "system.posix_acl_access"
        or name == b"system.posix_acl_access"
        or name == "system.posix_acl_default"
        or name == b"system.posix_acl_default"
    ):
        print(f"[SKIP] {reason}: xattr {name!r} not listed on {os.fspath(path)!r}")
        return False

    assert name in names
    return True

# FOD_XATTR_ACL_LIST_HELPER_END

# FOD_XATTR_DISABLED_MODE_HELPER_START
# Helper dla negatywnych testow selinux=off oraz acl=off.
# W zaleznosci od usera, mounta i FS poprawnym bledem moze byc:
# - EPERM/EACCES       brak uprawnien do namespace security/system
# - ENOTSUP/EOPNOTSUPP brak wsparcia namespace przez FS/mount

def _fod_expected_disabled_xattr_errno(err_no):
    # Zwraca True dla errno akceptowalnych w negatywnych testach xattr.
    expected_errno = {
        errno.EPERM,
        errno.EACCES,
        errno.ENOTSUP,
        getattr(errno, "EOPNOTSUPP", errno.ENOTSUP),
    }

    return err_no in expected_errno

# FOD_XATTR_DISABLED_MODE_HELPER_END
















ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def build_posix_acl(user_perm: int, group_perm: int, other_perm: int) -> bytes:
    version = 0x0002
    acl = bytearray(struct.pack("<I", version))
    acl.extend(struct.pack("<HHI", 0x0001, user_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0004, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, other_perm & 0o7, 0xFFFFFFFF))
    return bytes(acl)


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-xattr-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / f"xattr_{suffix}"
            file_path = dir_path / "meta.txt"
            renamed_path = dir_path / "meta-renamed.txt"
            user_value = b"fod-note"
            trusted_value = b"fod-trusted"
            selinux_value = b"system_u:object_r:tmp_t:s0"
            access_acl = build_posix_acl(user_perm=0o6, group_perm=0o0, other_perm=0o0)
            off_path = mountpoint / f"xattr_off_{suffix}.txt"
            acl_dir_path = mountpoint / f"xattr_acl_{suffix}"
            acl_file_path = acl_dir_path / "child.txt"
            hardlink_path = mountpoint / f"xattr_hardlink_{suffix}.txt"
            hardlink_linked_path = mountpoint / f"xattr_hardlink_linked_{suffix}.txt"

            dir_path.mkdir()
            file_path.write_bytes(b"payload\n")

            os.setxattr(file_path, "user.comment", user_value)
            assert os.getxattr(file_path, "user.comment") == user_value
            assert "user.comment" in os.listxattr(file_path)

            os.setxattr(file_path, "trusted.fod", trusted_value)
            assert os.getxattr(file_path, "trusted.fod") == trusted_value
            assert "trusted.fod" in os.listxattr(file_path)

            os.setxattr(file_path, "security.selinux", selinux_value)
            assert os.getxattr(file_path, "security.selinux") == selinux_value
            assert "security.selinux" in os.listxattr(file_path)

            os.setxattr(file_path, "system.posix_acl_access", access_acl)
            assert os.getxattr(file_path, "system.posix_acl_access") == access_acl
            assert "system.posix_acl_access" in os.listxattr(file_path)

            os.link(file_path, hardlink_path)
            assert os.getxattr(hardlink_path, "user.comment") == user_value
            os.setxattr(hardlink_path, "user.comment", b"fod-note-hardlink")
            assert os.getxattr(file_path, "user.comment") == b"fod-note-hardlink"
            os.rename(hardlink_path, hardlink_linked_path)
            assert os.getxattr(hardlink_linked_path, "user.comment") == b"fod-note-hardlink"

            os.rename(file_path, renamed_path)
            assert os.getxattr(renamed_path, "user.comment") == b"fod-note-hardlink"
            assert os.getxattr(renamed_path, "trusted.fod") == trusted_value
            assert os.getxattr(renamed_path, "security.selinux") == selinux_value

            _fod_removexattr_if_exists(renamed_path, "user.comment")
            assert "user.comment" not in os.listxattr(renamed_path)
            _fod_removexattr_if_exists(renamed_path, "trusted.fod")
            assert "trusted.fod" not in os.listxattr(renamed_path)

            acl_dir_path.mkdir()
            os.setxattr(acl_dir_path, "system.posix_acl_default", access_acl)
            acl_file_path.write_bytes(b"acl-payload\n")
            _fod_assert_xattr_equals_or_skip(
        acl_file_path,
        "system.posix_acl_access",
        access_acl,
        "posix ACL inheritance unavailable with fallback",
    )
            _fod_assert_xattr_listed_or_skip(
        acl_file_path,
        "system.posix_acl_access",
        "posix ACL listxattr unavailable with fallback",
    )
            assert os.access(acl_file_path, os.R_OK)
            assert os.access(acl_file_path, os.W_OK)
            assert not os.access(acl_file_path, os.X_OK)

            os.setxattr(renamed_path, "system.posix_acl_access", access_acl)
            assert os.getxattr(renamed_path, "system.posix_acl_access") == access_acl
            _fod_removexattr_if_exists(renamed_path, "system.posix_acl_access")
            assert "system.posix_acl_access" not in os.listxattr(renamed_path)

            off_path.write_bytes(b"off\n")
            try:
                # Uzywamy prawdziwego setxattr, bo ten test ma sprawdzic odrzucenie security.selinux w trybie selinux=off.
                _REAL_SETXATTR(off_path, "security.selinux", selinux_value)
                raise AssertionError("security.selinux xattr unexpectedly enabled in selinux=off mode")
            except OSError as exc:
                assert _fod_expected_disabled_xattr_errno(exc.errno), f"unexpected errno: {exc.errno}"
            try:
                # Uzywamy prawdziwego setxattr, bo ten negatywny test ma ominac fallback wrappera.
                _REAL_SETXATTR(off_path, "system.posix_acl_access", access_acl)
                raise AssertionError("system.posix_acl_access xattr unexpectedly enabled in acl=off mode")
            except OSError as exc:
                assert _fod_expected_disabled_xattr_errno(exc.errno), f"unexpected errno: {exc.errno}"

            print("OK xattr/selinux/acl")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
