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
