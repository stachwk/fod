#!/usr/bin/env python
# -*- coding: utf-8 -*-

import argparse
import sys
from pathlib import Path


def print_err(message):
    print(f"stderr: {message}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(
        description="FOD 3.4.30: run runtime-profile recovery standby on the primary PostgreSQL image."
    )
    parser.add_argument("--repo", default=str(Path.home() / "git" / "fod"))
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()

    repo = Path(args.repo).expanduser().resolve()
    version_path = repo / "fod_version.txt"
    test_path = repo / "tests/integration/test_runtime_profile.py"

    for path in (version_path, test_path):
        if not path.is_file():
            print_err(f"missing required file: {path}")
            return 2

    if version_path.read_text(encoding="utf-8").strip() != "3.4.30":
        print_err("patch requires FOD 3.4.30")
        return 3

    text = test_path.read_text(encoding="utf-8")

    if "def _docker_container_image(" in text:
        print_err("standby image patch already applied")
        return 4

    network_anchor = """def _docker_primary_network(container_name: str = "fod-postgres") -> str:
    result = _docker(
        [
            "inspect",
            "-f",
            "{{range $k, $_ := .NetworkSettings.Networks}}{{println $k}}{{end}}",
            container_name,
        ]
    )
    networks = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if not networks:
        raise RuntimeError(f"could not determine Docker network for {container_name}")
    return networks[0]


"""

    network_replacement = """def _docker_primary_network(container_name: str = "fod-postgres") -> str:
    result = _docker(
        [
            "inspect",
            "-f",
            "{{range $k, $_ := .NetworkSettings.Networks}}{{println $k}}{{end}}",
            container_name,
        ]
    )
    networks = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if not networks:
        raise RuntimeError(f"could not determine Docker network for {container_name}")
    return networks[0]


def _docker_container_image(container_name: str = "fod-postgres") -> str:
    result = _docker(
        [
            "inspect",
            "-f",
            "{{.Config.Image}}",
            container_name,
        ]
    )
    image = result.stdout.strip()
    if not image:
        raise RuntimeError(f"could not determine Docker image for {container_name}")
    return image


"""

    wait_anchor = """    while time.monotonic() < deadline:
        try:
            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                transaction_read_only = cur.fetchone()[0]
                cur.execute("SELECT pg_is_in_recovery()")
                is_in_recovery = cur.fetchone()[0]
                if is_in_recovery and transaction_read_only == "on":
                    return
        except Exception as exc:  # pragma: no cover - failure path only
            last_error = exc
        time.sleep(0.5)
"""

    wait_replacement = """    while time.monotonic() < deadline:
        state = _docker(
            [
                "inspect",
                "-f",
                "{{.State.Status}} {{.State.ExitCode}}",
                container_name,
            ],
            check=False,
        )
        state_text = state.stdout.strip()
        if state.returncode == 0 and (
            state_text.startswith("exited ")
            or state_text.startswith("dead ")
        ):
            logs = _docker(["logs", container_name], check=False)
            raise AssertionError(
                "recovery standby exited before becoming ready:\\n"
                f"state={state_text}\\n"
                f"stdout:\\n{logs.stdout}\\n"
                f"stderr:\\n{logs.stderr}"
            )

        try:
            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                transaction_read_only = cur.fetchone()[0]
                cur.execute("SELECT pg_is_in_recovery()")
                is_in_recovery = cur.fetchone()[0]
                if is_in_recovery and transaction_read_only == "on":
                    return
        except Exception as exc:  # pragma: no cover - failure path only
            last_error = exc
        time.sleep(0.5)
"""

    network_use_anchor = """            primary_network = _docker_primary_network()
            standby_data_path = Path(standby_data_dir.name)
"""

    network_use_replacement = """            primary_network = _docker_primary_network()
            primary_image = _docker_container_image()
            standby_data_path = Path(standby_data_dir.name)
"""

    image_anchor = '''                    "postgres:16-alpine",
'''

    image_replacement = """                    primary_image,
"""

    anchors = (
        ("network helper", network_anchor),
        ("recovery wait", wait_anchor),
        ("primary network use", network_use_anchor),
        ("standby image", image_anchor),
    )

    for name, anchor in anchors:
        count = text.count(anchor)
        if count != 1:
            print_err(f"{name} anchor count={count}, expected=1")
            return 5

    text = text.replace(network_anchor, network_replacement, 1)
    text = text.replace(wait_anchor, wait_replacement, 1)
    text = text.replace(network_use_anchor, network_use_replacement, 1)
    text = text.replace(image_anchor, image_replacement, 1)

    test_path.write_text(text, encoding="utf-8")

    if args.verbose:
        print_err(
            "runtime-profile recovery standby now reuses the primary container image "
            "and fails immediately with logs if the standby exits"
        )

    print("OK: recovery standby uses primary PostgreSQL image")
    print("OK: recovery wait reports early container exit")
    print("OK: version remains 3.4.30")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
