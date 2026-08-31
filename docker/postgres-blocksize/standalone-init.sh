#!/bin/sh
set -eu

# Intentionally empty. The standalone/default FOD PostgreSQL service must not
# inherit benchmark-only replication role/bootstrap behavior from older image
# revisions. Benchmark compose files mount their own primary-init.sh explicitly.
