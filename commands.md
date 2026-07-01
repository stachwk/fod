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
