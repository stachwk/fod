#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""C10: read-only inspection of PostgreSQL privileges bypassing FOD fencing."""

from __future__ import annotations

import argparse
import os
import sys

import psycopg2

SENSITIVE_TABLES = (
    "files",
    "data_objects",
    "data_blocks",
    "file_write_leases",
    "destination_write_leases",
    "client_sessions",
)
WRITE_PRIVILEGES = ("INSERT", "UPDATE", "DELETE", "TRUNCATE")


def yesno(value: bool) -> str:
    return "YES" if value else "NO"


def database_connection():
    return psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST", "127.0.0.1"),
        port=os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT", "5432"),
        dbname=os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB", "foddbname"),
        user=os.environ.get("FOD_PG_USER") or os.environ.get("POSTGRES_USER", "foduser"),
        password=os.environ.get("FOD_PG_PASSWORD") or os.environ.get("POSTGRES_PASSWORD", ""),
        connect_timeout=5,
    )


def audit(connection) -> tuple[str, list[str]]:
    # Set the transaction read-only before examining user and table privileges.
    connection.set_session(readonly=True, autocommit=False)
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT current_database(), current_user, session_user,
                   r.rolsuper, r.rolcreatedb, r.rolcreaterole, r.rolbypassrls,
                   d.datdba = r.oid AS owns_database
              FROM pg_catalog.pg_roles r
              JOIN pg_catalog.pg_database d ON d.datname = current_database()
             WHERE r.rolname = current_user
            """
        )
        row = cursor.fetchone()
        if row is None:
            raise RuntimeError("current database role was not found in pg_roles")
        database, role, session_role, superuser, createdb, createrole, bypassrls, owns_db = row
        print(
            f"C10_ROLE database={database} role={role} session_role={session_role} "
            f"superuser={yesno(superuser)} createdb={yesno(createdb)} "
            f"createrole={yesno(createrole)} bypassrls={yesno(bypassrls)} "
            f"owns_database={yesno(owns_db)}"
        )

        cursor.execute(
            """
            SELECT n.nspname,
                   pg_catalog.pg_get_userbyid(n.nspowner) AS schema_owner,
                   pg_catalog.has_schema_privilege(current_user, n.oid, 'USAGE') AS can_use,
                   pg_catalog.has_schema_privilege(current_user, n.oid, 'CREATE') AS can_create
              FROM pg_catalog.pg_namespace n
             WHERE n.nspname = 'fod'
            """
        )
        schema_row = cursor.fetchone()
        if schema_row is None:
            raise RuntimeError("fod schema is missing; initialize FOD schema first")
        _, schema_owner, can_use, can_create = schema_row
        print(
            f"C10_SCHEMA schema=fod owner={schema_owner} "
            f"usage={yesno(can_use)} create={yesno(can_create)}"
        )

        cursor.execute(
            """
            SELECT c.relname,
                   pg_catalog.pg_get_userbyid(c.relowner) AS table_owner,
                   pg_catalog.has_table_privilege(current_user, c.oid, 'INSERT') AS can_insert,
                   pg_catalog.has_table_privilege(current_user, c.oid, 'UPDATE') AS can_update,
                   pg_catalog.has_table_privilege(current_user, c.oid, 'DELETE') AS can_delete,
                   pg_catalog.has_table_privilege(current_user, c.oid, 'TRUNCATE') AS can_truncate
              FROM pg_catalog.pg_class c
              JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'fod'
               AND c.relkind IN ('r', 'p')
               AND c.relname = ANY(%s)
             ORDER BY c.relname
            """,
            (list(SENSITIVE_TABLES),),
        )
        rows = cursor.fetchall()
        found = {row[0] for row in rows}
        missing = sorted(set(SENSITIVE_TABLES) - found)
        bypass = False
        reasons = []
        if superuser:
            bypass = True
            reasons.append("superuser")
        if missing:
            reasons.append("missing_tables")
        for table, table_owner, *flags in rows:
            enabled = [p for p, granted in zip(WRITE_PRIVILEGES, flags) if granted]
            if enabled:
                bypass = True
                reasons.append(f"table_dml:{table}")
            print(
                f"C10_TABLE schema=fod table={table} owner={table_owner} "
                f"write_privileges={','.join(enabled) if enabled else 'NONE'}"
            )
        if missing:
            print(f"C10_MISSING_TABLES={','.join(missing)}")
        print(
            f"C10_DIRECT_DML_BYPASS={yesno(bypass)} "
            f"checked_tables={len(rows)}/{len(SENSITIVE_TABLES)}"
        )
        if bypass:
            print(
                "C10_LIMITATION=runtime role can directly mutate authoritative FOD "
                "tables without invoking Rust fencing validation"
            )
        connection.rollback()
        return ("UNRESTRICTED" if bypass else "RESTRICTED"), reasons


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Read-only PostgreSQL role/privilege inventory for FOD C10."
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="return nonzero if the current role can directly mutate FOD state",
    )
    args = parser.parse_args()
    try:
        connection = database_connection()
    except psycopg2.Error as error:
        print(f"C10_AUDIT=ERROR stage=connect type={type(error).__name__}", file=sys.stderr)
        return 3
    try:
        status, reasons = audit(connection)
    except (psycopg2.Error, RuntimeError) as error:
        print(f"C10_AUDIT=ERROR stage=query type={type(error).__name__}", file=sys.stderr)
        return 3
    finally:
        connection.close()

    print(f"C10_AUDIT={status} reason={','.join(reasons) if reasons else 'none'}")
    if args.strict and status != "RESTRICTED":
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
