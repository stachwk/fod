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
