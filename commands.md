# Commands

## 2026-07-01

Base commit at execution time: `f2e531b`

- `rg -n "materialize.*rollback|rollback.*materialize|materialize_cleaned|plan.*completed|completed" tests rust_indexer rust_hotpath rust_fuse -S`
- `sed -n '360,620p' rust_indexer/src/materialize.rs`
- `sed -n '1,280p' tests/integration/test_fod_indexer_materialize_rollback.py`
- `sed -n '1,360p' rust_indexer/src/cleanup.rs`
- `python3 -m py_compile tests/integration/test_fod_indexer_materialize_rollback.py`
- `make test-fod-indexer-materialize-rollback`

Base commit at execution time: `7bcfc5d`

- `rg -n "postgres-benchmarks-wal-preset|POSTGRES_BENCHMARK_REPEAT|postgres-benchmarks" Makefile . -S`
- `sed -n '1090,1165p' Makefile`
- `sed -n '1,120p' tests/integration/test_postgresql_wal_pressure.py`
- `sed -n '1,120p' tests/integration/fod_postgres_benchmark.py`
- `POSTGRES_BENCHMARK_REPEAT=0 make postgres-benchmarks-wal-preset`
- `make -n POSTGRES_BENCHMARK_REPEAT=2 postgres-benchmarks-wal-preset`
- `make help`

Base commit at execution time: `53b30d1`

- `sed -n '1,240p' /home/wojtek/.codex/attachments/e8504898-5964-40bc-9686-03b49ddef26f/pasted-text.txt`
- `sed -n '241,520p' /home/wojtek/.codex/attachments/e8504898-5964-40bc-9686-03b49ddef26f/pasted-text.txt`
- `sed -n '521,900p' /home/wojtek/.codex/attachments/e8504898-5964-40bc-9686-03b49ddef26f/pasted-text.txt`
- `sed -n '901,1300p' /home/wojtek/.codex/attachments/e8504898-5964-40bc-9686-03b49ddef26f/pasted-text.txt`
- `python3 -m py_compile tests/integration/test_fod_indexer_usability.py`
- `make test-fod-indexer-usability`
- `make test-fod-indexer-plan-import-scope`
- `make test-fod-indexer-cleanup-failed`
- `make test-fod-indexer-plan-import-scope && make test-fod-indexer-cleanup-failed`

Base commit at execution time: `b619fb5`

- `rg -n "CARGO_RUN|FOD_.*DEBUG_BIN|^init:|^init-qnap:|^reset:|^config-show:|^indexer:|^indexer-import:|^mount:|^mount-qnap:|^mount-user:|^demo:" Makefile`
- `sed -n '1,90p' Makefile`
- `sed -n '520,720p' Makefile`
- `make -n build-debug`
- `make build-debug`
- `make init`
- `make indexer INDEXER_ARGS='--help'`
- `test -e /dev/fuse && test -r /dev/fuse && test -w /dev/fuse; echo fuse_device=$?`
- `command -v fusermount3 || command -v fusermount || true`
- `mountpoint -q /tmp/fod-mount; echo mounted=$?`
- `timeout 12s make mount`
- `mountpoint -q /tmp/fod-mount; echo mounted=$?`
- `make test-fod-indexer-materialize-rollback`

Base commit at execution time: `597b185`

- `rg -n "test-runtime-profile|runtime_profile|CARGO_BUILD_MKFS|CARGO_BUILD_FUSE" Makefile tests/integration/test_runtime_profile.py tests/integration/fod_mount.py -S`
- `sed -n '1060,1090p' Makefile`
- `rg -n "def run_fod_change|fod-change|FOD_CHANGE" tests/integration/fod_runtime_testlib.py -S`
- `make -n test-runtime-profile`
- `make -n change-runtime-list`
- `make build-debug && make test-runtime-profile`
- `make change-runtime-list`

Base commit at execution time: `9d3f255`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/8de54ae6-1430-4108-b525-05bd3cb3b015/pasted-text.txt`
- `sed -n '261,520p' /home/wojtek/.codex/attachments/8de54ae6-1430-4108-b525-05bd3cb3b015/pasted-text.txt`
- `rg -n "CARGO_BUILD_MKFS|CARGO_BUILD_FUSE|CARGO_BUILD_INDEXER|CARGO_RUN_MKFS|CARGO_RUN_INDEXER|cargo run|cargo build" Makefile`
- `make -n build-debug`
- `make -n test-multi-open-unique-handles && make -n test-copy-block-crc-table`
- `make build-debug && make init && make indexer INDEXER_ARGS='--help' && make test-fod-indexer-materialize-rollback && make test-multi-open-unique-handles && make test-copy-block-crc-table`
- `make -n docker-selinux-acl-smoke`
- `make test-runtime-profile`
- `make change-runtime-list`

Base commit at execution time: `453896f`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/b3be45ea-82c6-427e-bfda-7a5d3cef960c/pasted-text.txt`
- `sed -n '261,520p' /home/wojtek/.codex/attachments/b3be45ea-82c6-427e-bfda-7a5d3cef960c/pasted-text.txt`
- `rg -n "^venv:|VENV_|ensurepip|pip install|^deps:|^clean:" Makefile`
- `sed -n '130,180p' Makefile`
- `sed -n '220,270p' Makefile`
- `make -n venv`
- `make venv && make -n venv`
- `make -n test-fod-indexer-materialize-rollback`
- `make -n test-runtime-profile`
- `test -f .venv/.fod-venv.stamp && ls -l .venv/.fod-venv.stamp requirements-test.txt`
- `make venv && make test-fod-indexer-materialize-rollback && make test-fod-indexer-usability && make test-runtime-profile && make test-fod-indexer-plan-import-scope`

Base commit at execution time: `669e2bd`

- `rg -n "minimal|minimaln|minimum|mał[aey]? zakres|keep.*small|small.*change|zmian[ay].*mał|zmian[ay].*minimal" --glob '*.md' .`
- `git rev-parse --short HEAD`
- `git status --short`
- `date -Is`
- `tail -n 60 conclusions.md`
- `tail -n 60 commands.md`
- `head -n 80 commands.md`
- `tail -n 40 commands.md`
- `tail -n 20 conclusions.md`
- `cat fod_version.txt`
- `git diff -- conclusions.md commands.md`
- `git add conclusions.md commands.md`
- `git commit -m 'FOD 3.2.1: record scope conclusion'`

Base commit at execution time: `3af1bda`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/370c8b40-d234-494a-9dea-50151c5da4dc/pasted-text.txt`
- `git rev-parse --short HEAD`
- `git status --short`
- `rg -n "^venv:|VENV_|ensurepip|pip install|requirements-test|\\.fod-venv\\.stamp|^deps:|^clean:" Makefile`
- `cat fod_version.txt`
- `sed -n '261,520p' /home/wojtek/.codex/attachments/370c8b40-d234-494a-9dea-50151c5da4dc/pasted-text.txt`
- `test -f requirements-test.txt && cat requirements-test.txt || true`
- `rg -n "incremental venv|venv setup|fod-venv|requirements-test|Python test virtualenv|parallel test isolation" TODO.md conclusions.md`
- `sed -n '140,175p' Makefile && sed -n '445,465p' Makefile && sed -n '1228,1238p' Makefile`
- `rg -n "ensurepip|pip install|requirements-test|VENV_STAMP" Makefile requirements-test.txt`
- `make -n venv`
- `make -n test-fod-indexer-materialize-rollback`
- `make -n test-runtime-profile`
- `make -n test-fod-indexer-usability`
- `make venv`
- `make -n venv`
- `make test-fod-indexer-materialize-rollback`
- `make test-fod-indexer-usability`
- `make test-runtime-profile`
- `make test-fod-indexer-plan-import-scope`
- `rg -n "ensurepip|pip install|requirements-test|VENV_STAMP" Makefile requirements-test.txt`
- `git status --short`
- `date -Is`
- `git rev-parse --short HEAD`
- `sed -n '1,55p' conclusions.md`
- `tail -n 40 commands.md`
- `sed -n '1,35p' TODO.md`
- `git diff -- conclusions.md commands.md`
- `git add conclusions.md commands.md`
- `git commit -m 'FOD 3.2.1: record venv verification'`

Base commit at execution time: `95e26b4`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/12547a85-324b-4c78-8fe9-ec04b0bb28d6/pasted-text.txt`
- `git rev-parse --short HEAD`
- `git status --short`
- `cat fod_version.txt`
- `sed -n '261,520p' /home/wojtek/.codex/attachments/12547a85-324b-4c78-8fe9-ec04b0bb28d6/pasted-text.txt`
- `ls tests/integration/test_fod_indexer_*.py`
- `rg -n "smoke|source|SOURCE|cleanup|DELETE FROM|TRUNCATE|shutil\\.rmtree|/tmp/fod-indexer|plan-import|materialize|index_sources|index_files|index_import" tests/integration/test_fod_indexer_*.py`
- `rg -n "CREATE TABLE index_sources|index_import_plans|index_import_plan_entries|index_files|index_file_hashes|index_duplicate_sets" migrations rust_mkfs rust_indexer -S`
- `sed -n '1,260p' tests/integration/test_fod_indexer_materialize.py`
- `sed -n '1,360p' tests/integration/test_fod_indexer_materialize_rollback.py`
- `sed -n '1,360p' tests/integration/test_fod_indexer_cleanup_failed.py`
- `sed -n '1,320p' tests/integration/test_fod_indexer_plan_import_scope.py`
- `sed -n '1,320p' tests/integration/fod_indexer_testlib.py`
- `sed -n '1,300p' tests/integration/test_fod_indexer_json_output.py`
- `sed -n '1,340p' tests/integration/test_fod_indexer_source_kinds.py`
- `sed -n '1,640p' tests/integration/test_fod_indexer_usability.py`
- `sed -n '270,370p' migrations/base_schema.sql`
- `sed -n '1,120p' migrations/0013_indexer.sql`
- `rg -n "test-fod-indexer|test-all|\\.PHONY" Makefile`
- `sed -n '640,700p' Makefile`
- `sed -n '210,310p' rust_indexer/src/hash.rs`
- `rg -n "report duplicates|duplicate_sets|source" rust_indexer/src/report.rs rust_indexer/src/cli.rs rust_indexer/src/hash.rs rust_indexer/src/plan.rs -S`
- `sed -n '1,140p' rust_indexer/src/report.rs && sed -n '1,140p' rust_indexer/src/hash.rs`
- `sed -n '160,260p' rust_indexer/src/plan.rs`
- `sed -n '300,385p' rust_indexer/src/plan.rs`
- `rg -n "struct Duplicate|DuplicateReport|human_readable|serde|members" rust_indexer/src -S`
- `sed -n '130,220p' rust_indexer/src/output.rs`
- `sed -n '1,260p' tests/integration/test_fod_indexer_materialize_rollback.py`
- `sed -n '260,420p' tests/integration/test_fod_indexer_materialize_rollback.py`
- `python3 -m py_compile tests/integration/test_fod_indexer_materialize_rollback.py`
- `tail -n 20 tests/integration/test_fod_indexer_materialize_rollback.py && python3 -m py_compile tests/integration/test_fod_indexer_materialize_rollback.py`
- `python3 -m py_compile tests/integration/test_fod_indexer_usability.py`
- `rg -n "cleanup_indexer_state|cleanup_materialized_roots\\(|/tmp/fod-indexer|\\\"smoke\\\"|json-smoke|ux-smoke|clean-smoke|local-smoke|mirror-smoke|github-smoke|adb-documents|rollback-smoke|rollback-completed-smoke" tests/integration/test_fod_indexer_*.py tests/integration/fod_indexer_testlib.py`
- `python3 -m py_compile tests/integration/test_fod_indexer_*.py tests/integration/fod_indexer_testlib.py`
- `rg -n "import shutil|shutil\\." tests/integration/test_fod_indexer_*.py`
- `rg -n "cleanup_indexer_state|cleanup_materialized_roots\\(|json-smoke|ux-smoke|clean-smoke|local-smoke|mirror-smoke|github-smoke|adb-documents|rollback-smoke|rollback-completed-smoke|--name\\s+smoke|\\\"smoke\\\"" tests/integration/test_fod_indexer_*.py`
- `python3 -m py_compile tests/integration/test_fod_indexer_*.py tests/integration/fod_indexer_testlib.py`
- `git diff --stat`
- `make -n test-fod-indexer-parallel-smoke`
- `make test-fod-indexer-materialize-rollback`
- `make test-fod-indexer-usability`
- `make test-fod-indexer-plan-import-scope`
- `make test-fod-indexer-cleanup-failed`
- `make test-fod-indexer-json-output`
- `make test-fod-indexer-smoke`
- `POSTGRES_DB=foddbname POSTGRES_USER=foduser POSTGRES_PASSWORD=cichosza POSTGRES_PORT=5432 .venv/bin/python tests/integration/test_fod_indexer_source_kinds.py`
- `make test-fod-indexer-parallel-smoke`
- `sed -n '1,24p' TODO.md`
- `sed -n '1,45p' conclusions.md`
- `tail -n 50 commands.md`
- `git status --short`
- `date -Is`
- `git diff --stat`
- `git diff --check`
- `python3 -m py_compile tests/integration/test_fod_indexer_*.py tests/integration/fod_indexer_testlib.py`
- `git diff -- Makefile TODO.md conclusions.md | sed -n '1,240p'`
- `git diff -- Makefile tests/integration/fod_indexer_testlib.py tests/integration/test_fod_indexer_materialize.py tests/integration/test_fod_indexer_materialize_rollback.py tests/integration/test_fod_indexer_cleanup_failed.py tests/integration/test_fod_indexer_plan_import_scope.py tests/integration/test_fod_indexer_json_output.py tests/integration/test_fod_indexer_usability.py tests/integration/test_fod_indexer_source_kinds.py TODO.md conclusions.md commands.md`
- `git add Makefile tests/integration/fod_indexer_testlib.py tests/integration/test_fod_indexer_materialize.py tests/integration/test_fod_indexer_materialize_rollback.py tests/integration/test_fod_indexer_cleanup_failed.py tests/integration/test_fod_indexer_plan_import_scope.py tests/integration/test_fod_indexer_json_output.py tests/integration/test_fod_indexer_usability.py tests/integration/test_fod_indexer_source_kinds.py TODO.md conclusions.md commands.md`
- `git commit -m 'FOD 3.2.1: isolate fod-indexer integration smokes'`

Base commit at execution time: `ad72bfc`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `git rev-parse --short HEAD`
- `git status --short`
- `cat fod_version.txt`
- `sed -n '261,620p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `rg -n "\[profile|postgres-benchmarks|pg_stat|bpftrace|perf" Cargo.toml Makefile docs scripts tests BENCHMARKS.md TODO.md conclusions.md -S`
- `find scripts -maxdepth 3 -type f -printf '%p\n'`
- `sed -n '1,120p' Cargo.toml && sed -n '920,980p' Makefile && sed -n '1130,1185p' Makefile`
- `sed -n '621,980p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `sed -n '120,210p' Makefile && sed -n '210,270p' Makefile && sed -n '700,735p' Makefile`
- `find docs -maxdepth 2 -type f -printf '%p\n' | sort`
- `rg -n "PSQL|psql -h|PGPASSWORD|FOD_PG_HOST|POSTGRES_PORT|COMPOSE_RUN" Makefile`
- `test -f .gitignore && sed -n '1,200p' .gitignore || true`
- `rg -n "artifacts|target|\.venv|perf" .gitignore .git/info/exclude 2>/dev/null || true`
- `sed -n '270,325p' TODO.md && sed -n '325,350p' TODO.md`
- `tail -n 80 commands.md`
- `mkdir -p scripts/perf/pg scripts/perf/bpftrace`
- `make -n profile-env`
- `make -n profile-pg-reset`
- `make -n profile-pg-top`
- `make -n profile-pg-wal`
- `make -n profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark`
- `make -n profile-perf-record PROFILE_WORKLOAD=test-large-copy-benchmark`
- `make -n profile-fuse-attach PROFILE_PID=12345`
- `make -n profile-indexer-attach PROFILE_PID=12345`
- `make -n profile-bpftrace-syscalls PROFILE_SECONDS=10`
- `make -n profile-local-baseline PROFILE_WORKLOAD=test-fod-indexer-materialize-rollback`
- `cargo build --profile profiling --workspace`
- `make build-debug`
- `make venv`
- `make init`
- `make profile-env`
- `make profile-pg-reset`
- `make test-fod-indexer-materialize-rollback`
- `make profile-pg-top`
- `make profile-pg-wal`
- `make -n profile-pg-activity`
- `make -n profile-pg-io`
- `test -f docs/performance.md`
- `command -v perf || true; command -v bpftrace || true`
- `make profile-pg-activity`
- `make profile-pg-io`
- `git status --short`
- `test -s artifacts/perf/ad72bfc/pg_io_stats.txt && { wc -l artifacts/perf/ad72bfc/pg_io_stats.txt; sed -n '1,80p' artifacts/perf/ad72bfc/pg_io_stats.txt; } || true`
- `find artifacts/perf/ad72bfc -maxdepth 1 -type f -printf '%p %s\n' 2>/dev/null || true`
- `grep -n "profile-pg-io" -A18 -B4 Makefile`
- `sed -n '1,260p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `sed -n '260,620p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `sed -n '1187,1275p' Makefile`
- `sed -n '1,260p' docs/performance.md`
- `sed -n '620,900p' /home/wojtek/.codex/attachments/1c7d5b4a-61cb-454d-ac12-1b2c4327abf2/pasted-text.txt`
- `make -n profile-bpftrace-read-hist PROFILE_SECONDS=10`
- `make -n profile-bpftrace-write-hist PROFILE_SECONDS=10`
- `git diff --stat && git diff -- Cargo.toml Makefile TODO.md conclusions.md docs/performance.md scripts/perf/pg/reset.sql scripts/perf/pg/top_statements.sql scripts/perf/pg/wal_checkpointer.sql scripts/perf/pg/io_stats.sql scripts/perf/pg/activity.sql scripts/perf/bpftrace/syscalls_by_comm.bt scripts/perf/bpftrace/read_size_hist.bt scripts/perf/bpftrace/write_size_hist.bt`
- `tail -n 120 commands.md`
- `sed -n '1,80p' conclusions.md`
- `git rev-parse --short HEAD && git rev-parse HEAD`
- `date -Is`
- `for f in scripts/perf/pg/*.sql scripts/perf/bpftrace/*.bt; do printf '\n== %s ==\n' "$f"; sed -n '1,200p' "$f"; done`
- `sed -n '1,80p' TODO.md`
- `rg -n "profile-|profiling|performance" Makefile docs/performance.md TODO.md conclusions.md scripts/perf -S`
- `git diff --check`
- `make -n profile-pg-activity`
- `make -n profile-pg-io`
- `test -f docs/performance.md && test -f scripts/perf/pg/reset.sql && test -f scripts/perf/bpftrace/syscalls_by_comm.bt`
- `rm -rf artifacts/perf/ad72bfc && rmdir -p artifacts/perf artifacts 2>/dev/null || true`
- `make -n profile-pg-io`
- `make profile-pg-io`
- `rm -rf artifacts/perf/ad72bfc && rmdir -p artifacts/perf artifacts 2>/dev/null || true`

Date: `2026-07-01`
Base commit at execution time: `8e8e95f`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/484dad60-c192-481b-9924-3eec20619898/pasted-text.txt`
- `sed -n '260,620p' /home/wojtek/.codex/attachments/484dad60-c192-481b-9924-3eec20619898/pasted-text.txt`
- `git status --short && git log -1 --oneline && cat fod_version.txt`
- `date -Is`
- `rg -n "profile-env|profile-local-baseline|profile-pg-top|profile-pg-wal|profile-perf-stat|profile-fuse-attach|ARTIFACTS_DIR" Makefile`
- `find scripts/perf -maxdepth 3 -type f -print | sort`
- `sed -n '1,180p' docs/performance.md`
- `sed -n '1,30p' TODO.md`
- `rg -n "artifacts|perf" .gitignore || true`
- `find artifacts -maxdepth 4 -type f -printf '%p %s\n' 2>/dev/null | sort || true`
- `sed -n '120,140p' Makefile`
- `sed -n '1,220p' .gitignore`
- `test -f docs/performance-baselines.md && sed -n '1,200p' docs/performance-baselines.md || true`
- `printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$(date -u +%Y%m%dT%H%M%SZ)" "$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown-host)"`
- `make build-debug && make venv && make init`
- `make profile-env PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make test-fod-indexer-materialize-rollback && make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make profile-pg-activity PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make profile-pg-io PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300`
- `make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make test-fod-indexer-materialize-rollback && make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback && make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback && make profile-pg-activity PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback && make profile-pg-io PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback`
- `make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make test-fod-indexer-plan-import-scope && make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=plan-import-scope && make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=plan-import-scope`
- `make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 && make test-large-copy-benchmark && make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy && make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy`
- `command -v perf || true`
- `make profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300`
- `find artifacts/perf -maxdepth 4 -type f -printf '%p %s\n' | sort`
- `find artifacts/perf/8e8e95f/lt7300-20260701T115956Z -maxdepth 1 -type f -printf '%f\n' | sort`
- `sed -n '1,80p' artifacts/perf/8e8e95f/lt7300-20260701T115956Z/env.txt`
- `test -f artifacts/perf/8e8e95f/lt7300-20260701T115956Z/perf-stat-test-large-copy-benchmark.txt && sed -n '1,120p' artifacts/perf/8e8e95f/lt7300-20260701T115956Z/perf-stat-test-large-copy-benchmark.txt || true`
- `for f in pg_top_statements-rollback.txt pg_top_statements-plan-import-scope.txt pg_top_statements-large-copy.txt; do echo "== $f =="; sed -n '1,12p' "artifacts/perf/8e8e95f/lt7300-20260701T115956Z/$f"; done`
- `for f in pg_wal_checkpointer-rollback.txt pg_wal_checkpointer-plan-import-scope.txt pg_wal_checkpointer-large-copy.txt; do echo "== $f =="; sed -n '1,28p' "artifacts/perf/8e8e95f/lt7300-20260701T115956Z/$f"; done`
- `sed -n '1,60p' artifacts/perf/8e8e95f/lt7300-20260701T115956Z/pg_io_stats-rollback.txt`
- `sed -n '1,40p' artifacts/perf/8e8e95f/lt7300-20260701T115956Z/perf-stat-test-large-copy-benchmark.txt`
- `make profile-local-baseline PROFILE_WORKLOAD=test-fod-indexer-materialize-rollback PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=validation`
- `PGPASSWORD="cichosza" psql -v ON_ERROR_STOP=1 -h 127.0.0.1 -p 5432 -U foduser -d foddbname -At -c "SELECT version();" -c "SHOW server_version_num;"`
- `git diff --check`
- `make -n profile-env`
- `make -n profile-local-baseline PROFILE_WORKLOAD=test-fod-indexer-materialize-rollback`
- `make -n profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark`
- `test -f docs/performance.md && test -f docs/performance-baselines.md && test -f scripts/perf/pg/reset.sql && test -f scripts/perf/bpftrace/syscalls_by_comm.bt`
- `perf stat failed: /usr/bin/perf is installed, but kernel perf_event_paranoid=4 blocks unprivileged counters.`
- `QNAP baseline skipped: this plan requested local PostgreSQL only.`
- `bpftrace real run skipped: not part of this local baseline and requires elevated host permissions.`
- `tail -n 100 commands.md`
- `git status --short --ignored | sed -n '1,80p'`
- `git diff --stat`
- `git diff --check`
- `git diff -- .gitignore Makefile docs/performance.md docs/performance-baselines.md TODO.md conclusions.md commands.md | sed -n '1,260p'`
- `sed -n '1,260p' docs/performance-baselines.md`
- `cat fod_version.txt && git status --short`
- `find artifacts/perf/8e8e95f/lt7300-20260701T115956Z -maxdepth 1 -type f -printf '%f %s\n' | sort`
- `git add .gitignore Makefile TODO.md commands.md conclusions.md docs/performance.md docs/performance-baselines.md`
- `git commit -m 'FOD 3.2.1: record first performance baseline'`

Date: `2026-07-01`
Base commit at execution time: `918f8b1`

- `git status --short && git log -1 --oneline && cat fod_version.txt`
- `date -Is && printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$(date -u +%Y%m%dT%H%M%SZ)" "$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown-host)"`
- `command -v perf || true; command -v bpftrace || true; cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true`
- `sudo -n true && echo sudo-n-ok || echo sudo-n-failed`
- `make profile-env PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 && make profile-pg-reset PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 && mkdir -p artifacts/perf/918f8b1/lt7300-20260701T120934Z && sudo -n perf stat -d -d -d -r 5 -o artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-test-large-copy-benchmark.txt -- make --no-print-directory test-large-copy-benchmark && make profile-pg-top PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy-sudo-perf && make profile-pg-wal PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy-sudo-perf`
- `sed -n '1,220p' artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-test-large-copy-benchmark.txt`
- `make profile-pg-reset PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 && sudo -n perf stat -d -d -d -r 3 -o artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-warm-test-large-copy-benchmark.txt -- make --no-print-directory test-large-copy-benchmark && make profile-pg-top PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy-sudo-perf-warm && make profile-pg-wal PROFILE_RUN_ID=20260701T120934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy-sudo-perf-warm`
- `sed -n '1,220p' artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-warm-test-large-copy-benchmark.txt`
- `sudo -n timeout 3s bpftrace scripts/perf/bpftrace/syscalls_by_comm.bt > artifacts/perf/918f8b1/lt7300-20260701T120934Z/bpftrace-syscalls-sudo-smoke.txt 2>&1; status=$?; echo bpftrace_status=$status; sed -n '1,120p' artifacts/perf/918f8b1/lt7300-20260701T120934Z/bpftrace-syscalls-sudo-smoke.txt; exit 0`
- `find artifacts/perf/918f8b1/lt7300-20260701T120934Z -maxdepth 1 -type f -printf '%f %s\n' | sort`
- `set -u; ART=artifacts/perf/918f8b1/lt7300-20260701T120934Z; mkdir -p "$ART"; sudo -n timeout 14s bpftrace scripts/perf/bpftrace/syscalls_by_comm.bt > "$ART/bpftrace-syscalls-sudo-large-copy.txt" 2>&1 & BPF_PID=$!; sleep 1; make --no-print-directory test-large-copy-benchmark; wait "$BPF_PID" || BPF_STATUS=$?; printf 'bpftrace_status=%s\n' "${BPF_STATUS:-0}" >> "$ART/bpftrace-syscalls-sudo-large-copy.txt"; sed -n '1,180p' "$ART/bpftrace-syscalls-sudo-large-copy.txt"`
- `find target -user root -maxdepth 4 -printf '%u:%g %p\n' 2>/dev/null | sed -n '1,80p'`
- `find artifacts/perf/918f8b1/lt7300-20260701T120934Z -user root -maxdepth 1 -printf '%u:%g %p\n' 2>/dev/null | sed -n '1,80p'`
- `stat -c '%U:%G %n' target target/debug target/debug/deps 2>/dev/null || true`
- `tail -n 40 artifacts/perf/918f8b1/lt7300-20260701T120934Z/bpftrace-syscalls-sudo-large-copy.txt 2>/dev/null || true`
- `sudo -n chown -R "$(id -u):$(id -g)" target artifacts/perf/918f8b1/lt7300-20260701T120934Z && { find target -user root -maxdepth 4 -printf '%u:%g %p\n' 2>/dev/null | sed -n '1,20p'; find artifacts/perf/918f8b1/lt7300-20260701T120934Z -user root -maxdepth 1 -printf '%u:%g %p\n' 2>/dev/null | sed -n '1,20p'; }`
- `set -u; ART=artifacts/perf/918f8b1/lt7300-20260701T120934Z; make --no-print-directory test-large-copy-benchmark >/tmp/fod-large-copy-warmup.log 2>&1; sudo -n timeout 14s bpftrace scripts/perf/bpftrace/syscalls_by_comm.bt > "$ART/bpftrace-syscalls-sudo-large-copy-warm.txt" 2>&1 & BPF_PID=$!; sleep 1; make --no-print-directory test-large-copy-benchmark; wait "$BPF_PID" || BPF_STATUS=$?; printf 'bpftrace_status=%s\n' "${BPF_STATUS:-0}" >> "$ART/bpftrace-syscalls-sudo-large-copy-warm.txt"; sed -n '1,220p' "$ART/bpftrace-syscalls-sudo-large-copy-warm.txt"`
- `set -u; ART=artifacts/perf/918f8b1/lt7300-20260701T120934Z; mkdir -p "$ART"; sudo -n perf stat -a -d -d -d -o "$ART/perf-stat-sudo-system-large-copy.txt" -- sleep 12 & PERF_PID=$!; sleep 1; make --no-print-directory test-large-copy-benchmark; wait "$PERF_PID" || PERF_STATUS=$?; printf 'perf_system_status=%s\n' "${PERF_STATUS:-0}" >> "$ART/perf-stat-sudo-system-large-copy.txt"; sed -n '1,220p' "$ART/perf-stat-sudo-system-large-copy.txt"`
- `sudo -n chown -R "$(id -u):$(id -g)" artifacts/perf/918f8b1/lt7300-20260701T120934Z target && { echo '== perf warm =='; sed -n '1,80p' artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-warm-test-large-copy-benchmark.txt; echo '== perf system =='; sed -n '1,80p' artifacts/perf/918f8b1/lt7300-20260701T120934Z/perf-stat-sudo-system-large-copy.txt; echo '== bpftrace fod/postgres snippets =='; grep -E '@\[(fod|postgres|large_copy|docker-proxy|docker|cargo|rustc)' artifacts/perf/918f8b1/lt7300-20260701T120934Z/bpftrace-syscalls-sudo-large-copy-warm.txt | tail -n 80; }`

Date: `2026-07-01`
Base commit at execution time: `0d047bb`

- `git status --short && git log -1 --oneline && cat fod_version.txt`
- `sed -n '120,145p' Makefile && sed -n '1187,1285p' Makefile`
- `sed -n '55,110p' docs/performance.md && sed -n '10,24p' TODO.md`
- `date -Is`
- `rg -n "^SHELL|.SHELLFLAGS" Makefile`
- `make -n profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T122000Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=helper PROFILE_SUDO='sudo -n'`
- `make -n profile-sudo-bpftrace-syscalls-workload PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T122000Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=helper PROFILE_SECONDS=5 PROFILE_SUDO='sudo -n'`
- `git diff --check`
- `git diff --stat`
- `make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T122000Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=helper PROFILE_SUDO='sudo -n'`
- `make profile-sudo-bpftrace-syscalls-workload PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T122000Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=helper PROFILE_SECONDS=8 PROFILE_SUDO='sudo -n'`
- `find target artifacts/perf/0d047bb/lt7300-20260701T122000Z -user root -maxdepth 4 -printf '%u:%g %p\n' 2>/dev/null | sed -n '1,80p'`
- `find artifacts/perf/0d047bb/lt7300-20260701T122000Z -maxdepth 1 -type f -printf '%f %s\n' 2>/dev/null | sort`
- `git status --short --ignored | sed -n '1,100p'`
- `git diff --check`
- `git add Makefile TODO.md conclusions.md docs/performance.md commands.md`
- `git commit -m 'FOD 3.2.1: add safe sudo profiling helpers'`

Base commit at execution time: `024547a`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/d9cb3817-38c8-4cdd-a19c-b78fdc6b7da2/pasted-text.txt`
- `sed -n '260,620p' /home/wojtek/.codex/attachments/d9cb3817-38c8-4cdd-a19c-b78fdc6b7da2/pasted-text.txt`
- `sed -n '620,920p' /home/wojtek/.codex/attachments/d9cb3817-38c8-4cdd-a19c-b78fdc6b7da2/pasted-text.txt`
- `rg -n "fod_persist_block_stage|data_blocks|COPY .*persist|ON CONFLICT|persist_block|PersistBlock|copy_in|copy_in_writer|binary" rust_hotpath rust_fuse rust_runtime rust_mkfs migrations tests -S`
- `rg -n "persist|data_blocks|block_stage|copy" rust_hotpath/src rust_fuse/src rust_runtime/src -S`
- `rg -n "CREATE TABLE.*data_blocks|CREATE INDEX.*data_blocks|UNIQUE.*data_blocks|data_blocks" migrations rust_mkfs rust_hotpath -S`
- `sed -n '1830,1895p' rust_hotpath/src/pg.rs`
- `sed -n '7370,7475p' rust_hotpath/src/pg.rs`
- `sed -n '7600,7765p' rust_hotpath/src/pg.rs`
- `rg -n "struct CopyInSession|impl CopyInSession|fn send\(" rust_hotpath/src/pg.rs`
- `sed -n '1,240p' rust_fuse/tests/large_copy_benchmark.rs`
- `PROFILE_RUN_ID=sql-persist-before-$(date +%Y%m%d-%H%M%S); echo "$PROFILE_RUN_ID" > /tmp/fod_sql_persist_before_run_id; echo "$PROFILE_RUN_ID"`
- `make build-debug`
- `make profile-env PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before`
- `make profile-pg-reset PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before`
- `make test-large-copy-benchmark`
- `make profile-pg-top PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before`
- `make profile-pg-wal PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before`
- `make profile-pg-activity PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before`
- `make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before PROFILE_SUDO='sudo -n'`
- `cargo fmt`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo test --manifest-path Cargo.toml -p fod-rust-hotpath --offline`
- `make build-debug`
- `PROFILE_RUN_ID=sql-persist-after-$(date +%Y%m%d-%H%M%S); echo "$PROFILE_RUN_ID" > /tmp/fod_sql_persist_after_run_id; echo "$PROFILE_RUN_ID"`
- `make profile-env PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after`
- `make profile-pg-reset PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after`
- `make test-large-copy-benchmark`
- `make profile-pg-top PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after`
- `make profile-pg-wal PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after`
- `for i in 1 2 3; do printf 'after_repeat=%s\n' "$i"; make test-large-copy-benchmark | rg 'OK large-copy-benchmark|test result'; done`
- `make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after PROFILE_SUDO='sudo -n'`
- `make profile-sudo-bpftrace-syscalls-workload PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after PROFILE_SECONDS=12 PROFILE_SUDO='sudo -n'`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark`
- `make test-fod-indexer-materialize`
- `make test-copy-block-crc-table`
- `make test-remount-durability-benchmark`
- `make test-persist-buffer-chunking`
- `make test-unlink-after-write`

Base commit at execution time: `a7f12f8`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/be96d8d6-8d74-4117-aba9-5d7690259dfe/pasted-text.txt`
- `sed -n '261,560p' /home/wojtek/.codex/attachments/be96d8d6-8d74-4117-aba9-5d7690259dfe/pasted-text.txt`
- `git status --short && git log -1 --oneline && cat fod_version.txt`
- `git show --stat --oneline --name-status HEAD`
- `rg -n "data-blocks|data_blocks|profile-pg-data-blocks" Makefile scripts/perf docs conclusions.md commands.md TODO.md -S`
- `ls -l scripts/perf/pg && sed -n '1,220p' scripts/perf/pg/data_blocks_semantics.sql 2>/dev/null`
- `sed -n '1,180p' docs/performance-profile-io-visibility-2026-07-01.md`
- `sed -n '1360,1410p' Makefile`
- `tail -n 80 conclusions.md`
- `tail -n 100 commands.md`
- `git log --oneline -8`
- `rg -n "DO NOTHING|FOD_DATA_BLOCKS_MERGE_DO_NOTHING|CRC rows after second copy" . -S`
- `git show --name-status --oneline 4bac9cf 2>/dev/null | sed -n '1,120p'`

Base commit at execution time: `4a66459`

- `sed -n '60,90p' migrations/base_schema.sql`
- `sed -n '350,370p' migrations/base_schema.sql`
- `rg -n "idx_data_blocks|CREATE TABLE data_blocks|data_blocks \\(" migrations rust_mkfs -S`
- `sed -n '1188,1228p' Makefile`
- `sed -n '235,260p' migrations/base_schema.sql && sed -n '1,70p' migrations/0005_data_objects.sql`
- `sed -n '320,365p' Makefile`
- `make profile-pg-data-blocks-merge-explain PROFILE_CAPTURE_LABEL=merge-explain`
- `nl -ba scripts/perf/pg/explain_data_blocks_merge.sql | sed -n '32,55p' && sed -n '1,120p' artifacts/perf/4a66459/lt7300-20260701T184148Z/pg_data_blocks_merge_explain-merge-explain.txt 2>/dev/null`
- `sed -n '1,80p' artifacts/perf/4a66459/lt7300-20260701T184214Z/pg_data_blocks_merge_explain-merge-explain.txt 2>/dev/null && PGPASSWORD=cichosza psql -v ON_ERROR_STOP=1 -h 127.0.0.1 -p 5432 -U foduser -d foddbname -c "SET search_path TO pg_temp, fod, public; CREATE TEMP TABLE tt (id int); CREATE INDEX tt_id ON tt(id);"`
- `printf '%s\n' "SET search_path TO pg_temp, fod, public;" "CREATE TEMP TABLE fod_explain_data_blocks (id_block INTEGER GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, id_file INTEGER NOT NULL, data_object_id INTEGER NOT NULL, _order INTEGER NOT NULL, data BYTEA NOT NULL) ON COMMIT DROP;" "\\dt pg_temp.*" "SELECT current_schema(), current_setting('search_path');" "CREATE INDEX fod_explain_data_blocks_data_object_id ON fod_explain_data_blocks (data_object_id);" | PGPASSWORD=cichosza psql -v ON_ERROR_STOP=1 -h 127.0.0.1 -p 5432 -U foduser -d foddbname`
- `make profile-pg-data-blocks-merge-explain PROFILE_CAPTURE_LABEL=merge-explain`
- `PROFILE_RUN_ID=merge-current-$(date -u +%Y%m%dT%H%M%SZ); PROFILE_HOST=$(hostname -s 2>/dev/null || hostname); printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_merge_current_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_merge_current_host; printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$PROFILE_RUN_ID" "$PROFILE_HOST"`
- `make profile-env PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-reset PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-merge-current.log`
- `make profile-pg-top PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-wal PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-data-blocks-semantics PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-data-blocks-merge-explain PROFILE_RUN_ID=merge-current-20260701T184307Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=merge-explain`
- `tail -n 140 docs/performance-baselines.md`
- `sed -n '1,40p' TODO.md`
- `rg -n "OK large-copy|pg.copy_put_data.aggregate" /tmp/fod-merge-current.log && find artifacts/perf/4a66459/lt7300-merge-current-20260701T184307Z -maxdepth 1 -type f -printf '%f %s\n' | sort`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `cat fod_version.txt`

- `git diff -- Makefile scripts/perf/pg/explain_data_blocks_merge.sql | sed -n '1,260p'`
- `git diff -- TODO.md conclusions.md docs/performance-baselines.md | sed -n '1,260p'`
- `git diff -- commands.md | sed -n '1,220p'`
- `sed -n '1,240p' scripts/perf/pg/explain_data_blocks_merge.sql`
- `make help | rg 'profile-pg-data-blocks-merge-explain|profile-pg-data-blocks-semantics'`
- `git add Makefile scripts/perf/pg/explain_data_blocks_merge.sql TODO.md commands.md conclusions.md docs/performance-baselines.md`
- `git commit -m 'FOD 3.2.1: add data block merge explain capture'`

Base commit at execution time: `8d5b1b6`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/35f5e54c-970c-4412-bca0-422f749ab2b9/pasted-text.txt`
- `sed -n '261,620p' /home/wojtek/.codex/attachments/35f5e54c-970c-4412-bca0-422f749ab2b9/pasted-text.txt`
- `git status --short && git log -5 --oneline && cat fod_version.txt`
- `git fetch origin`
- `rg -n "FOD_DATA_BLOCKS_MERGE_DO_NOTHING|data_blocks_merge_do_nothing_enabled" rust_hotpath/src/pg.rs || true`
- `git status --short && git log -5 --oneline`
- `PROFILE_RUN_ID=merge-current-$(date -u +%Y%m%dT%H%M%SZ); PROFILE_HOST=$(hostname -s 2>/dev/null || hostname); printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_merge_current_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_merge_current_host; printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$PROFILE_RUN_ID" "$PROFILE_HOST"`
- `make profile-env PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current && make profile-pg-reset PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-merge-current.log`
- `make profile-pg-top PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-wal PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-data-blocks-semantics PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `make profile-pg-data-blocks-merge-explain PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `sed -n '1,170p' Makefile`
- `sed -n '1220,1265p' Makefile`
- `sed -n '1,160p' scripts/perf/pg/wal_checkpointer.sql`
- `rg -n "^PROFILE_|ARTIFACTS_DIR|PROFILE_CAPTURE_SUFFIX|PSQL" Makefile`
- `make profile-pg-data-blocks-bloat PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `make help | rg 'profile-pg-data-blocks-bloat|profile-pg-data-blocks-merge-explain'`
- `git add Makefile scripts/perf/pg/data_blocks_bloat.sql commands.md conclusions.md docs/performance-baselines.md`
- `git commit -m 'FOD 3.2.1: add data block bloat diagnostics'`

Base commit at execution time: `5ca6f1e`

- `PROFILE_RUN_ID=wal-current-$(date -u +%Y%m%dT%H%M%SZ); PROFILE_HOST=$(hostname -s 2>/dev/null || hostname); printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_wal_current_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_wal_current_host; printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$PROFILE_RUN_ID" "$PROFILE_HOST"`
- `make profile-env PROFILE_RUN_ID=wal-current-20260701T185834Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=wal && make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185834Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-wal-current.log`
- `make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185834Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=after && make profile-pg-wal-delta PROFILE_RUN_ID=wal-current-20260701T185834Z PROFILE_HOST=lt7300`
- `PROFILE_RUN_ID=wal-current-$(date -u +%Y%m%dT%H%M%SZ); PROFILE_HOST=$(hostname -s 2>/dev/null || hostname); printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_wal_current_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_wal_current_host; printf 'PROFILE_RUN_ID=%s\nPROFILE_HOST=%s\n' "$PROFILE_RUN_ID" "$PROFILE_HOST"`
- `make profile-env PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=wal && make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-wal-current.log`
- `make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=after && make profile-pg-wal-delta PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300`
- `make help | rg 'profile-pg-wal-snapshot|profile-pg-wal-delta'`
- `rg -n "OK large-copy|pg.copy_put_data.aggregate" /tmp/fod-wal-current.log && cat artifacts/perf/5ca6f1e/lt7300-wal-current-20260701T185934Z/pg_wal_delta-before-to-after.tsv`
- `find artifacts/perf/5ca6f1e/lt7300-wal-current-20260701T185934Z -maxdepth 1 -type f -printf '%f %s\n' | sort`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `sed -n '1,140p' scripts/perf/pg/wal_snapshot.sql && make help | rg 'profile-pg-wal-snapshot|profile-pg-wal-delta'`
- `git add Makefile scripts/perf/pg/wal_snapshot.sql TODO.md commands.md conclusions.md docs/performance-baselines.md`
- `git commit -m 'FOD 3.2.1: capture data block WAL amplification'`

Base commit at execution time: `2659c1b`

- `git status --short`
- `find artifacts/perf -maxdepth 2 -type d -name '*copy-send-buffer*' | sort`
- `ls -l /tmp/fod-copy-send-buffer-default.log /tmp/fod-copy-send-buffer-smaller.log /tmp/fod-copy-send-buffer-larger.log`
- `git rev-parse --short HEAD`
- `find artifacts/perf/2659c1b/lt7300-copy-send-buffer-default-20260701T190336Z -maxdepth 1 -type f -printf '%f\n' | sort`
- `find artifacts/perf/2659c1b/lt7300-copy-send-buffer-smaller-20260701T190359Z -maxdepth 1 -type f -printf '%f\n' | sort`
- `find artifacts/perf/2659c1b/lt7300-copy-send-buffer-larger-20260701T190408Z -maxdepth 1 -type f -printf '%f\n' | sort`
- `rg -n "OK large-copy-benchmark|FOD_IO_AGGREGATE|FAILED|ERROR" /tmp/fod-copy-send-buffer-default.log /tmp/fod-copy-send-buffer-smaller.log /tmp/fod-copy-send-buffer-larger.log`
- `sed -n '1,220p' /tmp/fod-copy-send-buffer-default.log`
- `sed -n '1,220p' /tmp/fod-copy-send-buffer-smaller.log`
- `sed -n '1,220p' /tmp/fod-copy-send-buffer-larger.log`
- `for f in artifacts/perf/2659c1b/lt7300-copy-send-buffer-*/pg_wal_delta-before-to-after.tsv; do printf '%s\n' "$f"; sed -n '1,40p' "$f"; done`
- `sed -n '1,240p' artifacts/perf/2659c1b/lt7300-copy-send-buffer-default-20260701T190336Z/pg_top_statements-default.txt`
- `sed -n '1,240p' artifacts/perf/2659c1b/lt7300-copy-send-buffer-smaller-20260701T190359Z/pg_top_statements-smaller.txt`
- `sed -n '1,240p' artifacts/perf/2659c1b/lt7300-copy-send-buffer-larger-20260701T190408Z/pg_top_statements-larger.txt`
- `git diff -- rust_hotpath/src/pg.rs`
- `sed -n '1,90p' TODO.md`
- `sed -n '1,180p' docs/performance-baselines.md`
- `head -n 40 commands.md`
- `head -n 30 conclusions.md`
- `cargo fmt --all`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `FOD_PROFILE_IO=1 make test-large-copy-benchmark`
- `make test-copy-block-crc-table`
- `make test-remount-durability-benchmark`
- `make test-persist-buffer-chunking`
- `make test-unlink-after-write`
- `make test-rust-hotpath-copy-dedupe`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `cat fod_version.txt`
- `git diff -- rust_hotpath/src/pg.rs`
- `git diff -- TODO.md conclusions.md docs/performance-baselines.md`
- `git diff -- commands.md`

Base commit at execution time: `85b3ee0`

- `git status --short`
- `git rev-parse --short HEAD`
- `rg -n "QNAP|qnap|test-large-copy-benchmark|profile-pg" Makefile docker-compose.yml docs scripts -S`
- `ls -la /home/wojtek/git/config && rg -n "QNAP|qnap|DOCKER_HOST|192\\.168\\.1\\.11|POSTGRES" /home/wojtek/git/config -S`
- `sed -n '1200,1250p' Makefile`
- `sed -n '1030,1045p' Makefile`
- `sed -n '1340,1415p' Makefile`
- `sed -n '190,240p' Makefile`
- `make qnap-config-show`
- `sed -n '480,525p' Makefile`
- `make qnap-smoke`
- `make QNAP=1 profile-env PROFILE_RUN_ID=qnap-copy-send-buffer-preflight-20260701T191240Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-env PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=default`
- `make QNAP=1 profile-pg-reset PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 make QNAP=1 test-large-copy-benchmark`
- `make QNAP=1 profile-pg-top PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=default`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=after`
- `make QNAP=1 profile-pg-wal-delta PROFILE_RUN_ID=qnap-copy-send-buffer-default-20260701T191302Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-env PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=smaller`
- `make QNAP=1 profile-pg-reset PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 FOD_PERSIST_COPY_SEND_BUFFER_BYTES=65536 make QNAP=1 test-large-copy-benchmark`
- `make QNAP=1 profile-pg-top PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=smaller`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=after`
- `make QNAP=1 profile-pg-wal-delta PROFILE_RUN_ID=qnap-copy-send-buffer-smaller-20260701T191346Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-env PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=larger`
- `make QNAP=1 profile-pg-reset PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 FOD_PERSIST_COPY_SEND_BUFFER_BYTES=4194304 make QNAP=1 test-large-copy-benchmark`
- `make QNAP=1 profile-pg-top PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=larger`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=after`
- `make QNAP=1 profile-pg-wal-delta PROFILE_RUN_ID=qnap-copy-send-buffer-larger-20260701T191428Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-env PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=default-warm`
- `make QNAP=1 profile-pg-reset PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=before`
- `FOD_PROFILE_IO=1 make QNAP=1 test-large-copy-benchmark`
- `make QNAP=1 profile-pg-top PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=default-warm`
- `make QNAP=1 profile-pg-wal-snapshot PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap PROFILE_CAPTURE_LABEL=after`
- `make QNAP=1 profile-pg-wal-delta PROFILE_RUN_ID=qnap-copy-send-buffer-default-warm-20260701T191517Z PROFILE_HOST=qnap`
- `find artifacts/perf/85b3ee0 -maxdepth 1 -type d -name 'qnap-*copy-send-buffer*' | sort`
- `rg -n "OK large-copy-benchmark|FOD I/O profile: op=pg.copy_put_data.aggregate" /tmp/fod-qnap-copy-send-buffer-default.log /tmp/fod-qnap-copy-send-buffer-smaller.log /tmp/fod-qnap-copy-send-buffer-larger.log /tmp/fod-qnap-copy-send-buffer-default-warm.log`
- `for f in artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-*/pg_wal_delta-before-to-after.tsv; do printf '%s\n' "$f"; sed -n '1,20p' "$f"; done`
- `for f in artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-default-20260701T191302Z/pg_top_statements-default.txt artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-default-warm-20260701T191517Z/pg_top_statements-default-warm.txt artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-smaller-20260701T191346Z/pg_top_statements-smaller.txt artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-larger-20260701T191428Z/pg_top_statements-larger.txt; do printf '%s\n' "$f"; rg -n "COPY fod_persist_block_stage|INSERT INTO data_blocks" "$f" | head -n 3; done`
- `cat artifacts/perf/85b3ee0/qnap-qnap-copy-send-buffer-default-warm-20260701T191517Z/env.txt | sed -n '1,40p'`
- `date -Is`
- `git log -1 --oneline`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `cat fod_version.txt`

Base commit at execution time: `c2ce42b`

- `sed -n '1,260p' /home/wojtek/.codex/attachments/ba5394c1-1281-430a-bd3f-b82ef6a24d27/pasted-text.txt`
- `sed -n '261,620p' /home/wojtek/.codex/attachments/ba5394c1-1281-430a-bd3f-b82ef6a24d27/pasted-text.txt`
- `git status --short && git log -5 --oneline && cat fod_version.txt`
- `rg -n "FOD_DATA_BLOCKS_MERGE_DO_NOTHING|data_blocks_merge_do_nothing_enabled|DO NOTHING" rust_hotpath/src/pg.rs || true`
- `sed -n '1,200p' scripts/perf/pg/wal_snapshot.sql`
- `sed -n '1228,1248p' Makefile`
- `rg -n "PROFILE_WAL|PROFILE_CAPTURE|ARTIFACTS_DIR|PROFILE_HOST|PROFILE_RUN_ID" Makefile | sed -n '1,120p'`
- `sed -n '120,155p' Makefile`
- `sed -n '1418,1436p' Makefile`
- `sed -n '1,140p' scripts/perf/pg/data_blocks_bloat.sql`
- `rg -n "^PYTHON|VENV_PYTHON" Makefile | head -n 20`
- `sed -n '270,365p' Makefile`
- `rg -n "pg_wal_delta|PROFILE_WAL_DELTA_FILE|profile-data-blocks-summary" . -S`
- `python3 -m py_compile scripts/perf/pg/wal_delta.py scripts/perf/summarize_data_blocks_profile.py`
- `cargo fmt --check`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `make profile-pg-data-blocks-bloat`

## 2026-07-03

Base commit at execution time: `a395cb8`

- `git rev-parse --short HEAD && git status --short`
- `which rust-analyzer || true; rust-analyzer --version || true; rustc --version; cargo --version; rustup show active-toolchain 2>/dev/null || true`
- `rg -n "rust-analyzer|checkOnSave|rust-src|sysroot|linkedProjects|procMacro|cargo|targetDir" -S . .vscode ~/.config/Code/User/settings.json ~/.config/Code\ -\ OSS/User/settings.json 2>/dev/null`
- `find . -maxdepth 3 \( -name 'rust-toolchain*' -o -name 'rust-project.json' -o -name '.rust-analyzer.toml' -o -path './.vscode/*' \) -print`
- `sed -n '1,120p' .vscode/settings.json`
- `test -d /usr/src/rustc-1.85.1/library && find /usr/src/rustc-1.85.1/library -maxdepth 2 -type f -name lib.rs | head -n 20 || true; test -d /usr/lib/rust-1.85 && find /usr/lib/rust-1.85 -maxdepth 3 -type d | sed -n '1,40p' || true`
- `find ~/.vscode ~/.vscode-server ~/.var/app ~/.local/share -path '*rust-analyzer*' \( -type f -o -type l \) 2>/dev/null | sed -n '1,80p'`
- `find ~/.vscode/extensions ~/.vscode-server/extensions -maxdepth 2 -type f -name package.json 2>/dev/null | xargs -r rg -n '"name": "rust-analyzer"|"version"|rust-lang.rust-analyzer' | sed -n '1,120p'`
- `find ~/.vscode/extensions/rust-lang.rust-analyzer-0.3.2955-linux-x64 -maxdepth 4 -type f -o -type l | sed -n '1,160p'`
- `find ~/.vscode/extensions/rust-lang.rust-analyzer-0.3.2955-linux-x64 -maxdepth 5 -type f -perm -111 -print -exec file {} \; | sed -n '1,120p'`
- `find ~/.cache ~/.config ~/.local/share -iname '*rust*analyzer*' -o -iname '*ra-*' 2>/dev/null | sed -n '1,120p'`
- `~/.vscode/extensions/rust-lang.rust-analyzer-0.3.2955-linux-x64/server/rust-analyzer --version || true`
- `sed -n '1,240p' /home/wojtek/.config/Code/logs/20260703T233425/window1/exthost/output_logging_20260703T233429/1-rust-analyzer\ Language\ Server.log`
- `sed -n '1,240p' /home/wojtek/.config/Code/logs/20260703T233425/window1/exthost/rust-lang.rust-analyzer/rust-analyzer\ Extension.log`
- `tail -120 /home/wojtek/.config/Code/logs/20260703T233425/window1/exthost/output_logging_20260703T233429/1-rust-analyzer\ Language\ Server.log`
- `~/.vscode/extensions/rust-lang.rust-analyzer-0.3.2955-linux-x64/server/rust-analyzer --help | sed -n '1,220p'`
- `git ls-files .vscode/settings.json .vscode/extensions.json .vscode/tasks.json .vscode/launch.json && git status --short`
- `sed -n '240,520p' /home/wojtek/.config/Code/logs/20260703T233425/window1/exthost/rust-lang.rust-analyzer/rust-analyzer\ Extension.log`
- `git check-ignore -v .vscode/settings.json .vscode/extensions.json .vscode/tasks.json .vscode/launch.json || true`
- `git status --ignored --short .vscode/settings.json .vscode/extensions.json .vscode/tasks.json .vscode/launch.json`
- `git ls-files commands.md conclusions.md TODO.md BENCHMARKS.md | sed -n '1,40p'`
- `timeout 90s env RUST_BACKTRACE=1 ~/.vscode/extensions/rust-lang.rust-analyzer-0.3.2955-linux-x64/server/rust-analyzer prime-caches . 2>&1 | tee /tmp/fod-rust-analyzer-prime-caches.log; printf 'exit=%s\n' "${PIPESTATUS[0]}"`
- `find /usr/lib/rust-1.85/lib/rustlib/src /usr/lib/rust-1.85/rustlib/src -maxdepth 4 -type d -name library -print 2>/dev/null; find /usr/lib/rust-1.85/lib/rustlib/src /usr/lib/rust-1.85/rustlib/src -maxdepth 5 -type f -name lib.rs | sed -n '1,60p'`
- `tail -40 /tmp/fod-rust-analyzer-prime-caches.log`
- `ls -ld /usr/src/rustc-1.85.1/library /usr/lib/rust-1.85/lib/rustlib/src/rust/library /usr/lib/rust-1.85/rustlib/src/rust/library 2>/dev/null || true`
- `jq empty .vscode/settings.json && sed -n '1,40p' .vscode/settings.json`
- `git status --short --ignored .vscode/settings.json commands.md conclusions.md`
- `git status --short`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `rg -n 'VS Code `rust-analyzer` panic|cachePriming|a395cb8|rust-analyzer-0.3.2955' conclusions.md commands.md | tail -n 80`
- `git add commands.md conclusions.md && git commit -m "FOD 3.2.1: record rust analyzer cache priming workaround"`

## 2026-07-03

Base commit at execution time: `778a805` with uncommitted working-tree changes for unchanged `data_blocks` conflict filtering.

- `git status --short`
- `git rev-parse --short HEAD && cat fod_version.txt`
- `tail -80 commands.md`
- `git diff --stat`
- `git diff -- rust_hotpath/src/pg.rs rust_fuse/tests/data_blocks_conflict_benchmark.rs Makefile docs/performance.md scripts/perf/pg/explain_data_blocks_merge.sql | sed -n '1,260p'`
- `PROFILE_RUN_ID="data-blocks-conflict-noop-smoke-$(date -u +%Y%m%dT%H%M%SZ)"; DATA_BLOCKS_CONFLICT_ID="$PROFILE_RUN_ID"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_conflict_noop_smoke_run_id; make profile-data-blocks-conflict-noop-dml PROFILE_RUN_ID="$PROFILE_RUN_ID" DATA_BLOCKS_CONFLICT_ID="$DATA_BLOCKS_CONFLICT_ID" DATA_BLOCKS_CONFLICT_BLOCK_SIZE=1M DATA_BLOCKS_CONFLICT_BLOCK_COUNT=2 PROFILE_DATA_BLOCKS_CONFLICT_NOOP_LOG=/tmp/fod-data-blocks-conflict-noop-smoke.log`
- `PROFILE_RUN_ID="data-blocks-conflict-changed-smoke-$(date -u +%Y%m%dT%H%M%SZ)"; DATA_BLOCKS_CONFLICT_ID="$PROFILE_RUN_ID"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_conflict_changed_smoke_run_id; make profile-data-blocks-conflict-dml PROFILE_RUN_ID="$PROFILE_RUN_ID" DATA_BLOCKS_CONFLICT_ID="$DATA_BLOCKS_CONFLICT_ID" DATA_BLOCKS_CONFLICT_BLOCK_SIZE=1M DATA_BLOCKS_CONFLICT_BLOCK_COUNT=2 PROFILE_DATA_BLOCKS_CONFLICT_LOG=/tmp/fod-data-blocks-conflict-changed-smoke.log`
- `cargo fmt --check`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_noop_smoke_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}"; printf 'ART=%s\n' "$ART"; sed -n '1,70p' "$ART/pg_table_dml_delta-before-to-after.txt"; sed -n '1,45p' "$ART/pg_wal_delta-before-to-after.tsv"; sed -n '1,35p' /tmp/fod-data-blocks-conflict-noop-smoke.log`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_changed_smoke_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}"; printf 'ART=%s\n' "$ART"; sed -n '1,70p' "$ART/pg_table_dml_delta-before-to-after.txt"; sed -n '1,45p' "$ART/pg_wal_delta-before-to-after.tsv"; sed -n '1,35p' /tmp/fod-data-blocks-conflict-changed-smoke.log`
- `git diff --check && git status --short`
- `tail -40 conclusions.md`
- `rg -n "non-HOT|unchanged|conflict|data_blocks|server-side COPY|HOT|heap rewrite|swap" TODO.md docs/*.md BENCHMARKS.md | head -n 80`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `git add Makefile commands.md docs/performance.md rust_fuse/tests/data_blocks_conflict_benchmark.rs rust_hotpath/src/pg.rs scripts/perf/pg/explain_data_blocks_merge.sql && git commit -m "FOD 3.2.1: skip unchanged data block conflict updates"`

Base commit at execution time: `76867aa`

- `PROFILE_RUN_ID="data-blocks-conflict-noop-$(date -u +%Y%m%dT%H%M%SZ)"; DATA_BLOCKS_CONFLICT_ID="$PROFILE_RUN_ID"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_conflict_noop_run_id; make profile-data-blocks-conflict-noop-dml PROFILE_RUN_ID="$PROFILE_RUN_ID" DATA_BLOCKS_CONFLICT_ID="$DATA_BLOCKS_CONFLICT_ID" PROFILE_DATA_BLOCKS_CONFLICT_NOOP_LOG=/tmp/fod-data-blocks-conflict-noop-current.log`
- `sed -n '1,220p' scripts/perf/summarize_data_blocks_profile.py`
- `sed -n '1,130p' BENCHMARKS.md`
- `sed -n '1,36p' TODO.md`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_noop_run_id); HOST=$(hostname -s 2>/dev/null || hostname); ART="artifacts/perf/$(git rev-parse --short HEAD)/${HOST}-${RUN_ID}"; python3 scripts/perf/summarize_data_blocks_profile.py --artifact-dir "$ART" --large-copy-log /tmp/fod-data-blocks-conflict-noop-current.log --pg-top "$ART/pg_top_statements-conflict-noop.txt" --wal-delta "$ART/pg_wal_delta-before-to-after.tsv" --table-dml-delta "$ART/pg_table_dml_delta-before-to-after.txt" --data-blocks-bloat "$ART/pg_data_blocks_bloat-conflict-noop.txt" --output docs/performance-data-blocks-conflict-noop-profile-2026-07-03.md --run-id "$RUN_ID" --host "$HOST" --conclusion 'The unchanged-block conflict filter avoided all data_blocks row rewrites for a 64 MiB same-payload overwrite: zero inserts, zero updates, zero dead tuples, and only minimal metadata WAL remained.' --next-candidate 'Keep the filter; next optimize the changed-payload full-overwrite case separately, likely through a data-object-level swap or another design that avoids non-HOT row rewrites without weakening correctness.'`
- `sed -n '1,220p' docs/performance-data-blocks-conflict-noop-profile-2026-07-03.md`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_noop_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}"; rg -n "COPY fod_persist_block_stage|INSERT INTO data_blocks" "$ART/pg_top_statements-conflict-noop.txt" | head -n 10`
- `git status --short`
- `sed -n '1,240p' scripts/perf/pg/explain_data_blocks_merge.sql`
- `rg -n "profile-pg-data-blocks-merge-explain|explain_data_blocks_merge" Makefile docs/performance.md docs/performance-baselines.md`
- `RUN_ID="data-blocks-merge-filter-explain-$(date -u +%Y%m%dT%H%M%SZ)"; HOST=$(hostname -s 2>/dev/null || hostname); printf '%s\n' "$RUN_ID" > /tmp/fod_data_blocks_merge_filter_explain_run_id; make profile-env PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST" PROFILE_CAPTURE_LABEL=merge-filter; make profile-pg-data-blocks-merge-explain PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST" PROFILE_CAPTURE_LABEL=merge-filter`
- `git rev-parse HEAD && cat /tmp/fod_data_blocks_conflict_noop_run_id && cat /tmp/fod_data_blocks_merge_filter_explain_run_id`
- `RUN_ID=$(cat /tmp/fod_data_blocks_merge_filter_explain_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}/pg_data_blocks_merge_explain-merge-filter.txt"; rg -n "Execution Time|Rows Removed by Conflict Filter|Tuples Inserted|Conflicting Tuples|Buffers:" "$ART"`
- `git diff --check`
- `git diff --stat`
- `git status --short`
- `sed -n '1,95p' BENCHMARKS.md && sed -n '18,26p' TODO.md && tail -12 conclusions.md`
- `git add BENCHMARKS.md TODO.md commands.md conclusions.md docs/performance-data-blocks-conflict-noop-profile-2026-07-03.md && git commit -m "FOD 3.2.1: record unchanged data block conflict profile"`

## 2026-07-03

Base commit at execution time: `d5c63e2`

- `git status --short && git log -5 --oneline && cat fod_version.txt`
- `rg -n "large_copy_benchmark|test-large-copy-benchmark|copy_file_range|FOD_PROFILE_IO|data_blocks" rust_fuse rust_hotpath tests Makefile scripts docs | head -n 200`
- `sed -n '1,260p' rust_fuse/tests/large_copy_benchmark.rs && sed -n '1,260p' rust_fuse/tests/support/mod.rs`
- `rg -n "test-large-copy|large-copy|benchmark" Makefile rust_fuse/tests Cargo.toml rust_fuse/Cargo.toml`
- `sed -n '1,420p' rust_fuse/tests/support.rs`
- `sed -n '1,140p' rust_fuse/tests/large_file_multiblock_benchmark.rs && sed -n '1,120p' rust_fuse/tests/remount_durability_benchmark.rs`
- `sed -n '1035,1062p' Makefile && sed -n '60,115p' Makefile && sed -n '460,480p' Makefile`
- `rg -n "OpenOptions|seek\\(|write_all|flush|sync_all|truncate|set_len|FOD_BOOTSTRAP_BIN" rust_fuse/tests -S`
- `git status --short`
- `nl -ba Makefile | sed -n '88,115p' && nl -ba Makefile | sed -n '146,162p' && nl -ba Makefile | sed -n '280,292p' && nl -ba Makefile | sed -n '354,366p' && nl -ba Makefile | sed -n '466,474p' && nl -ba Makefile | sed -n '1045,1060p' && nl -ba Makefile | sed -n '1208,1216p' && nl -ba Makefile | sed -n '1270,1296p'`
- `nl -ba docs/performance.md | sed -n '45,70p'`
- `cargo fmt --check`
- `make -n test-data-blocks-conflict-seed && make -n test-data-blocks-conflict-overwrite-benchmark && make -n profile-data-blocks-conflict-dml PROFILE_RUN_ID=conflict-dry DATA_BLOCKS_CONFLICT_ID=conflict-dry`
- `make help | rg -n "data-blocks-conflict|profile-pg-table-dml"`
- `git diff --check && git status --short && git diff --stat`
- `cargo fmt`
- `git diff -- docs/performance.md Makefile rust_fuse/tests/support.rs rust_fuse/tests/data_blocks_conflict_benchmark.rs rust_fuse/tests/large_copy_benchmark.rs rust_fuse/tests/large_file_multiblock_benchmark.rs | sed -n '1,260p'`
- `cargo fmt --check && cargo check --manifest-path Cargo.toml -p fod-rust-fuse && git diff --check`
- `make -n profile-data-blocks-conflict-dml PROFILE_RUN_ID=conflict-dry DATA_BLOCKS_CONFLICT_ID=conflict-dry | sed -n '1,220p'`
- `git diff --stat && git status --short`
- `sed -n '1,120p' rust_fuse/tests/data_blocks_conflict_benchmark.rs && sed -n '52,72p' docs/performance.md`
- `PROFILE_RUN_ID="data-blocks-conflict-smoke-$(date -u +%Y%m%dT%H%M%SZ)"; DATA_BLOCKS_CONFLICT_ID="$PROFILE_RUN_ID"; make profile-data-blocks-conflict-dml PROFILE_RUN_ID="$PROFILE_RUN_ID" DATA_BLOCKS_CONFLICT_ID="$DATA_BLOCKS_CONFLICT_ID" DATA_BLOCKS_CONFLICT_BLOCK_SIZE=1M DATA_BLOCKS_CONFLICT_BLOCK_COUNT=2 PROFILE_DATA_BLOCKS_CONFLICT_LOG=/tmp/fod-data-blocks-conflict-smoke.log`
- `cargo fmt --check && cargo check --manifest-path Cargo.toml -p fod-rust-fuse && git diff --check`
- `make -n profile-data-blocks-conflict-dml PROFILE_RUN_ID=conflict-dry DATA_BLOCKS_CONFLICT_ID=conflict-dry | sed -n '1,220p'`
- `git diff --stat && git status --short`
- `sed -n '1,120p' rust_fuse/tests/data_blocks_conflict_benchmark.rs && sed -n '52,72p' docs/performance.md`
- `make test-large-copy-benchmark`
- `git diff --check && git status --short`
- `git diff --stat`
- `cat fod_version.txt`
- `git add Makefile commands.md docs/performance.md rust_fuse/tests/support.rs rust_fuse/tests/large_copy_benchmark.rs rust_fuse/tests/large_file_multiblock_benchmark.rs rust_fuse/tests/data_blocks_conflict_benchmark.rs && git commit -m "FOD 3.2.1: add data block conflict update benchmark"`

Base commit at execution time: `1969674`

- `PROFILE_RUN_ID="data-blocks-conflict-$(date -u +%Y%m%dT%H%M%SZ)"; DATA_BLOCKS_CONFLICT_ID="$PROFILE_RUN_ID"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_conflict_run_id; printf '%s\n' "$DATA_BLOCKS_CONFLICT_ID" > /tmp/fod_data_blocks_conflict_id; make profile-data-blocks-conflict-dml PROFILE_RUN_ID="$PROFILE_RUN_ID" DATA_BLOCKS_CONFLICT_ID="$DATA_BLOCKS_CONFLICT_ID" PROFILE_DATA_BLOCKS_CONFLICT_LOG=/tmp/fod-data-blocks-conflict-current.log`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}"; printf 'ART=%s\n' "$ART"; sed -n '1,80p' "$ART/pg_table_dml_delta-before-to-after.txt"; sed -n '1,45p' "$ART/pg_wal_delta-before-to-after.tsv"; sed -n '1,40p' /tmp/fod-data-blocks-conflict-current.log`
- `RUN_ID=$(cat /tmp/fod_data_blocks_conflict_run_id); ART="artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s 2>/dev/null || hostname)-${RUN_ID}"; rg -n "COPY fod_persist_block_stage|INSERT INTO data_blocks" "$ART/pg_top_statements-conflict.txt" | head -n 5; sed -n '1,80p' "$ART/env.txt"`
- `git status --short && git log -3 --oneline && cat fod_version.txt`
- `git diff --check && git status --short`
- `git diff --stat && sed -n '1,180p' docs/performance-data-blocks-conflict-profile-2026-07-03.md`
- `sed -n '1,80p' BENCHMARKS.md && sed -n '18,26p' TODO.md && tail -n 10 conclusions.md`
- `git add BENCHMARKS.md TODO.md commands.md conclusions.md docs/performance-data-blocks-conflict-profile-2026-07-03.md && git commit -m "FOD 3.2.1: record data block conflict profile"`

## 2026-07-03

Base commit at execution time: `d5150c3`

- `git status --short && git log -5 --oneline && cat fod_version.txt`
- `sed -n '120,170p' Makefile && sed -n '1220,1285p' Makefile && sed -n '1410,1455p' Makefile`
- `sed -n '1,220p' scripts/perf/pg/wal_delta.py && sed -n '1,180p' scripts/perf/pg/wal_snapshot.sql && rg -n "profile-pg|data-blocks|wal_delta|snapshot" Makefile scripts/perf docs README.md TODO.md conclusions.md`
- `tail -n 80 commands.md && tail -n 50 conclusions.md && sed -n '1,80p' TODO.md`
- `nl -ba Makefile | sed -n '130,165p'`
- `nl -ba Makefile | sed -n '330,365p'`
- `nl -ba Makefile | sed -n '1200,1270p'`
- `nl -ba scripts/perf/summarize_data_blocks_profile.py | sed -n '1,220p'`
- `nl -ba scripts/perf/pg/data_blocks_bloat.sql | sed -n '1,200p'`
- `rg -n "^PYTHON|^PSQL|VENV_PYTHON" Makefile | head -n 30`
- `rg -n "pg_stat_force_next_flush|table_dml|n_tup_hot|n_tup_ins|n_tup_upd" . -S`
- `ls -la scripts/perf/pg && find scripts/perf -maxdepth 2 -type f | sort`
- `nl -ba docs/performance.md | sed -n '1,90p'`
- `python3 -m py_compile scripts/perf/pg/metric_snapshot.py scripts/perf/pg/wal_delta.py scripts/perf/pg/table_dml_delta.py scripts/perf/summarize_data_blocks_profile.py`
- `git diff --check`
- `make -n profile-pg-table-dml-snapshot PROFILE_CAPTURE_LABEL=before && make -n profile-pg-table-dml-delta`
- `git diff --stat && git diff -- Makefile scripts/perf/pg/wal_delta.py scripts/perf/pg/table_dml_delta.py scripts/perf/pg/table_dml_snapshot.sql scripts/perf/summarize_data_blocks_profile.py docs/performance.md | sed -n '1,280p'`
- `git status --short`
- `python3 -m py_compile scripts/perf/pg/metric_snapshot.py scripts/perf/pg/wal_delta.py scripts/perf/pg/table_dml_delta.py scripts/perf/summarize_data_blocks_profile.py && git diff --check`
- `make profile-pg-table-dml-snapshot PROFILE_RUN_ID=dml-snapshot-smoke-$(date -u +%Y%m%dT%H%M%SZ) PROFILE_HOST=$(hostname -s 2>/dev/null || hostname) PROFILE_CAPTURE_LABEL=smoke`
- `make help | rg -n "profile-pg-table-dml|profile-pg-wal|profile-data-blocks-summary"`
- `SNAP="$(find artifacts/perf/d5150c3 -path '*dml-snapshot-smoke-*' -name 'pg_table_dml_snapshot-smoke.txt' | sort | tail -n 1)"; make profile-pg-table-dml-delta PROFILE_TABLE_DML_BEFORE_FILE="$SNAP" PROFILE_TABLE_DML_AFTER_FILE="$SNAP" PROFILE_RUN_ID=dml-delta-smoke-$(date -u +%Y%m%dT%H%M%SZ) PROFILE_HOST=$(hostname -s 2>/dev/null || hostname)`
- `tail -n 40 commands.md`
- `date -Is && git rev-parse --short HEAD && git status --short`

Base commit at execution time: `c5d7f24`

- `git add Makefile commands.md docs/performance.md scripts/perf/pg/metric_snapshot.py scripts/perf/pg/table_dml_delta.py scripts/perf/pg/table_dml_snapshot.sql scripts/perf/pg/wal_delta.py scripts/perf/summarize_data_blocks_profile.py && git commit -m "FOD 3.2.1: add data block DML delta profiling"`
- `PROFILE_RUN_ID="data-blocks-dml-$(date -u +%Y%m%dT%H%M%SZ)"; PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_dml_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_data_blocks_dml_host; make profile-env PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-reset PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-table-dml-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=before; make profile-pg-wal-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=before; FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-data-blocks-dml-current.log; make profile-pg-table-dml-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=after; make profile-pg-table-dml-delta PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-wal-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=after; make profile-pg-wal-delta PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-top PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=dml; make profile-pg-data-blocks-bloat PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=dml`
- `PROFILE_RUN_ID=$(cat /tmp/fod_data_blocks_dml_run_id); PROFILE_HOST=$(cat /tmp/fod_data_blocks_dml_host); make profile-data-blocks-summary PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=dml PROFILE_LARGE_COPY_LOG=/tmp/fod-data-blocks-dml-current.log PROFILE_TABLE_DML_DELTA_FILE="artifacts/perf/$(git rev-parse --short HEAD)/${PROFILE_HOST}-${PROFILE_RUN_ID}/pg_table_dml_delta-before-to-after.txt" PROFILE_DATA_BLOCKS_SUMMARY_OUTPUT=docs/performance-data-blocks-dml-profile-2026-07-03.md PROFILE_DATA_BLOCKS_SUMMARY_CONCLUSION='The real local large-copy path inserted 32768 data_blocks rows with zero data_blocks UPDATE/HOT/dead-tuple growth; this run measures insert-heavy COPY plus conflict lookup, not a conflict-update heap rewrite case.' PROFILE_DATA_BLOCKS_SUMMARY_NEXT='Add or run a targeted overwrite/conflict workload if the next question is HOT update eligibility for real data_blocks rewrites; keep production SQL unchanged until that separate update-heavy evidence exists.'`
- `sed -n '1,220p' docs/performance-data-blocks-dml-profile-2026-07-03.md`
- `sed -n '90,160p' BENCHMARKS.md && tail -n 30 BENCHMARKS.md`
- `git status --short && git diff --stat`
- `sed -n '16,28p' TODO.md && tail -n 20 conclusions.md`
- `sed -n '1,90p' BENCHMARKS.md`
- `git diff --check && git status --short`
- `git diff --stat && git diff -- BENCHMARKS.md TODO.md conclusions.md commands.md docs/performance-data-blocks-dml-profile-2026-07-03.md | sed -n '1,260p'`
- `git log -3 --oneline && cat fod_version.txt`
- `git add BENCHMARKS.md TODO.md commands.md conclusions.md docs/performance-data-blocks-dml-profile-2026-07-03.md && git commit -m "FOD 3.2.1: record data block DML delta profile"`
- `git status --short`
- `git diff --check`
- `git diff --stat`
- `git diff -- docs/performance-data-blocks-profile-2026-07-01.md conclusions.md TODO.md scripts/perf/summarize_data_blocks_profile.py | sed -n '1,260p'`
- `sed -n '1,220p' docs/performance-data-blocks-profile-2026-07-01.md`
- `RUN_ID="wal-delta-tool-smoke-$(date -u +%Y%m%dT%H%M%SZ)"; HOST=$(hostname -s 2>/dev/null || hostname); make profile-env PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST" PROFILE_CAPTURE_LABEL=tool-smoke && make profile-pg-wal-snapshot PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST" PROFILE_CAPTURE_LABEL=before && make profile-pg-wal-snapshot PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST" PROFILE_CAPTURE_LABEL=after && make profile-pg-wal-delta PROFILE_RUN_ID="$RUN_ID" PROFILE_HOST="$HOST"`

Base commit at execution time: `ac47828`

- `PROFILE_RUN_ID="data-blocks-current-$(date -u +%Y%m%dT%H%M%SZ)"; PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)"; printf '%s\n' "$PROFILE_RUN_ID" > /tmp/fod_data_blocks_current_run_id; printf '%s\n' "$PROFILE_HOST" > /tmp/fod_data_blocks_current_host; make profile-env PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-reset PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST"; make profile-pg-wal-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=before; FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee /tmp/fod-data-blocks-current.log; make profile-pg-wal-snapshot PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=after; make profile-pg-wal-delta PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current; make profile-pg-top PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current; make profile-pg-wal PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current; make profile-pg-data-blocks-semantics PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current; make profile-pg-data-blocks-bloat PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current; make profile-pg-data-blocks-merge-explain PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current`
- `PROFILE_RUN_ID=$(cat /tmp/fod_data_blocks_current_run_id); PROFILE_HOST=$(cat /tmp/fod_data_blocks_current_host); make profile-data-blocks-summary PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current PROFILE_LARGE_COPY_LOG=/tmp/fod-data-blocks-current.log PROFILE_DATA_BLOCKS_SUMMARY_CONCLUSION='The real local path still shows server-side COPY plus data_blocks merge as the dominant cost; WAL is measurable but checkpoints did not interfere in this run.' PROFILE_DATA_BLOCKS_SUMMARY_NEXT='Run COPY send buffer matrix and keep runtime SQL unchanged until repeated local/QNAP data identifies a stable next bottleneck.'`
- `for value in 262144 1048576 4194304 16777216; do run_id="copy-buffer-${value}-$(date -u +%Y%m%dT%H%M%SZ)"; make profile-env PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL="buffer-${value}"; make profile-pg-reset PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)"; make profile-pg-wal-snapshot PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL=before; FOD_PERSIST_COPY_SEND_BUFFER_BYTES="$value" FOD_PROFILE_IO=1 make test-large-copy-benchmark 2>&1 | tee "/tmp/fod-copy-buffer-${value}.log"; make profile-pg-wal-snapshot PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL=after; make profile-pg-wal-delta PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL="buffer-${value}"; make profile-pg-top PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL="buffer-${value}"; make profile-pg-data-blocks-bloat PROFILE_RUN_ID="$run_id" PROFILE_HOST="$(hostname -s 2>/dev/null || hostname)" PROFILE_CAPTURE_LABEL="buffer-${value}"; done`
- `find artifacts/perf/ac47828 -maxdepth 1 -type d -name 'lt7300-copy-buffer-*' | sort`
- `rg -n "OK large-copy-benchmark|FOD I/O profile: op=pg.copy_put_data.aggregate" /tmp/fod-copy-buffer-262144.log /tmp/fod-copy-buffer-1048576.log /tmp/fod-copy-buffer-4194304.log /tmp/fod-copy-buffer-16777216.log`
- `for f in artifacts/perf/ac47828/lt7300-copy-buffer-*/pg_wal_delta-before-to-after.tsv; do printf '%s\n' "$f"; sed -n '1,24p' "$f"; done`
- `sed -n '1,220p' docs/performance-data-blocks-profile-2026-07-01.md`
- `PROFILE_RUN_ID=$(cat /tmp/fod_data_blocks_current_run_id); PROFILE_HOST=$(cat /tmp/fod_data_blocks_current_host); make profile-data-blocks-summary PROFILE_RUN_ID="$PROFILE_RUN_ID" PROFILE_HOST="$PROFILE_HOST" PROFILE_CAPTURE_LABEL=current PROFILE_LARGE_COPY_LOG=/tmp/fod-data-blocks-current.log PROFILE_DATA_BLOCKS_SUMMARY_CONCLUSION='The real local path still shows server-side COPY plus data_blocks merge as the dominant cost; WAL is measurable but checkpoints did not interfere in this run.' PROFILE_DATA_BLOCKS_SUMMARY_NEXT='Run COPY send buffer matrix and keep runtime SQL unchanged until repeated local/QNAP data identifies a stable next bottleneck.'`
- `for f in artifacts/perf/ac47828/lt7300-copy-buffer-*/pg_top_statements-buffer-*.txt; do printf '%s\n' "$f"; rg -n "COPY fod_persist_block_stage|INSERT INTO data_blocks" "$f" | head -n 3; done`
- `for f in artifacts/perf/ac47828/lt7300-copy-buffer-*/pg_data_blocks_bloat-buffer-*.txt; do printf '%s\n' "$f"; rg -n "\\| data_blocks\\s+\\||^ data_blocks\\s+\\||idx_data_blocks_object_order" "$f"; done`
- `python3 -m py_compile scripts/perf/pg/wal_delta.py scripts/perf/summarize_data_blocks_profile.py`
- `cargo fmt --check`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `make profile-pg-data-blocks-bloat`

Base commit at execution time: `4cdaffb`

- `git status --short`
- `git diff -- rust_hotpath/src/pg.rs`
- `git rev-parse --short HEAD && cat fod_version.txt`
- `tail -80 commands.md`
- `tail -100 conclusions.md`
- `rg -n "data block|data_blocks|overwrite|swap|HOT|non-HOT|profile" TODO.md BENCHMARKS.md docs -g '*.md'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '3000,3185p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '7350,8240p'`
- `rg -n "fn file_data_object_info_on_conn|fn file_data_object_id_on_conn|fn detach_shared_data_object_on_conn|fn update_file_sizes_on_conn|fn persist_file_blocks" rust_hotpath/src/pg.rs`
- `rg -n "BEGIN|COMMIT|ROLLBACK|with_transaction|exec_command\\(conn" rust_hotpath/src/pg.rs | head -n 120`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '2840,3275p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '7687,7828p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '7829,8195p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '8195,8275p'`
- `rg -n "persist_file_blocks_with_crc_flag\\(|persist_file_blocks_from_path\\(|maintain_copy_crc_table|enable_extents|PersistExtent" rust_hotpath/src/pg.rs rust_fuse -S`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '3234,3365p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '2045,2185p'`
- `rg -n "replayable_sql_error|Replay|REPLAY|with_cached_connection|retry" rust_hotpath/src/pg.rs | head -n 200`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '2500,2618p' && nl -ba rust_hotpath/src/pg.rs | sed -n '9800,10020p'`
- `rg -n "data_object_id|block_map|load_block|load_blocks|fetch_block|read_block" rust_hotpath/src/pg.rs rust_fuse/src -S`
- `rg -n "file_data_object_id\\(|data_object_id\\(" rust_hotpath/src/pg.rs rust_fuse/src -S`
- `rg -n "cache|ReadBlockCache|read_cache|recent_write_blocks|write_state" rust_fuse/src rust_hotpath/src/pg.rs -S`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '3358,3485p' && nl -ba rust_fuse/src/read_cache.rs | sed -n '450,515p' && nl -ba rust_fuse/src/write_buffer.rs | sed -n '150,205p'`
- `nl -ba rust_hotpath/src/pg.rs | sed -n '360,575p'`
- `cargo fmt --check`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `cargo fmt`
- `cargo fmt --check`
- `cargo check --manifest-path Cargo.toml -p fod-rust-hotpath`
- `cargo check --manifest-path Cargo.toml -p fod-rust-fuse`
- `cargo test --manifest-path Cargo.toml -p fod-rust-hotpath recognizes_replayable_command_sql_for_disconnect_retry -- --nocapture`
- `git diff --check`
- `git diff --stat`
- `git diff -- rust_hotpath/src/pg.rs`
- `date -Is && git rev-parse --short HEAD && git status --short`
