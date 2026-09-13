# P2 QNAP PostgreSQL baseline follow-up

Status: completed 2026-09-12.

P2 required repeated evidence before changing
`FOD_PERSIST_COPY_SEND_BUFFER_BYTES` on the QNAP reference backend.

## Environment

- FOD: `3.4.23`
- measured commit: `0280127`
- client: `lt7300`
- QNAP reference host: 8 GB RAM, 2 CPU, HDD
- PostgreSQL: 16.15
- PostgreSQL `BLCKSZ`: 32 KiB
- FOD schema: 24

The QNAP schema was upgraded to version 24 before the final benchmark. This was
a non-destructive schema migration and was required because the benchmark's
`init` prerequisite correctly refused to initialize over an existing older FOD
schema.

## Benchmark

The final repeatability matrix used the real `test-large-copy-benchmark` with a
64 MiB payload (`4M * 16`).

Candidates:

- `default`
- `262144`
- `1048576`
- `4194304`

One warm-up was excluded. The measured matrix used five repetitions per
candidate and rotated candidate order between repetitions.

All 20 measured benchmark invocations passed.

## Results

| buffer bytes | mean MiB/s | median MiB/s | sample stdev | CV | paired wins vs default |
| --- | ---: | ---: | ---: | ---: | ---: |
| `default` | `10.646` | `10.48` | `0.592` | `5.6%` | baseline |
| `262144` | `10.182` | `10.35` | `1.349` | `13.3%` | `1/5` |
| `1048576` | `9.134` | `9.44` | `1.137` | `12.5%` | `0/5` |
| `4194304` | `8.852` | `9.77` | `1.741` | `19.7%` | `1/5` |

For `4194304` versus `default`:

- median: `-6.77%`;
- mean: `-16.85%`;
- paired wins: `1/5`;
- coefficient of variation: `19.7%`.

## Decision

No runtime tuning change is justified.

Keep the current `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` default unchanged.
Do not promote `4194304` to the default.

The earlier single-run QNAP improvement for 4 MiB was not repeatable under the
five-repeat controlled matrix. The current default was both faster overall and
more stable.

No FOD version bump is required because P2 produced no runtime code or default
configuration change.

The next active implementation priority is P3: external unmount/session
teardown.
