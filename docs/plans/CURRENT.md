# FOD current implementation plan

Updated: 2026-10-27. This is the single implementation queue. Work is performed
on `main`. Implemented behavior belongs in [CURRENT_STATE.md](../CURRENT_STATE.md);
long-term proposals belong in [ROADMAP.md](../../ROADMAP.md).

## Implementation order

| Priority | Work | Current boundary and completion evidence |
| --- | --- | --- |
| 1 | Primary-authoritative mutation decisions | The [authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md) records verified unlink lifetime, atomic rmdir, cross-type creation, transactional chmod/chown and sticky-bit checks. Default ACL inheritance, create authorization and parent ACL checks for unlink/rmdir/rename now share the relevant PostgreSQL mutation transaction. Generic xattr set/remove authorization and `CREATE`/`REPLACE` semantics now share the locked inode transaction. Mounted ACL/xattr validation remains open. Root `getattr` projects its PostgreSQL row and migration 0028 updates parent/root timestamps transactionally for namespace changes. Timestamp `setattr` now authorizes and preserves fields in one primary transaction; path `truncate` repeats authorization in its content transaction under the file row lock. Partial-write/truncate-boundary inputs, write/copy planning sizes and copy source/dedupe blocks now load from primary. Explicit telemetry/control DSNs are checked against the data cluster and database before read-only session/lease registration. Next: audit remaining mutation preconditions and validate ACL/xattr behavior on a mounted filesystem. |
| 2 | TX03J cross-mount read coherence | The [coherence specification](../TX03J_CROSS_MOUNT_CACHE_COHERENCE.md) defines the contract; the [generation audit](../tx03j-generation-audit.md) distinguishes implemented PostgreSQL generation primitives from missing cross-host integration. A strict opt-in read path and local cache gate do not close default multi-host coherence. |
| 3 | Degraded read-only recovery | The [HA implementation plan](HA_READONLY_RECOVERY_AND_CACHE.md) owns the detailed phases and acceptance matrix. Explicit standby reads and role monitoring exist; live source switching, fencing proof validation and rejoin barriers remain open. Use controlled remount until those barriers are complete. |
| 4 | Controlled cache performance comparison | Follow the performance workstream in the [HA/cache plan](HA_READONLY_RECOVERY_AND_CACHE.md#workstream-b-cache-and-historical-performance-comparison), after the relevant correctness gates. Existing five-run pilot measurements are diagnostic evidence, not a release baseline. |
| 5 | Quantify clock-skew and queue-order evidence | The [time requirements](../POSTGRESQL_REQUIREMENTS.md#3-synchronizacja-czasu-hostów) require monitored NTP synchronization. The shortest effective lock lease is 1 s; validate 100 ms inter-node offset as a candidate against PostgreSQL failover and fencing before adopting a numeric limit. FIFO tickets order waiters within each process, but production telemetry does not correlate tickets with task identities. If historical task order must be diagnosed, add ticket/task correlation locally and authoritative transaction-order evidence across hosts; wall-clock log sorting is insufficient. |

These priorities do not imply a diagnosed root cause beyond the evidence in
the linked reviews. Recheck source and existing regressions before choosing a
fix. Source-review suspicions must stay distinct from reproduced defects.

## Shared acceptance rules

- Namespace mutations, leases, fencing, inode generations and cross-host
  ordering remain PostgreSQL-authoritative under the
  [architecture invariants](../FOD_ARCHITECTURE_INVARIANTS.md).
- HA role observation alone cannot admit writes. Recovery requires trusted
  identity, timeline and deployment fencing evidence, plus request draining,
  cache invalidation and fresh authoritative leases.
- Retain deterministic reproductions and the exact build profile/toolchain.
  Unexplained intermittent defects require ASM/ELF analysis as specified in
  [AGENTS.md](../../AGENTS.md).
- Follow [development](../DEVELOPMENT.md),
  [versioning](../versioning.md) and
  [validation procedures](../../zasady_sprawdzen.md). After every commit,
  inspect the complete parent-to-commit diff.

## Deferred work

Reopen `readdirplus`, QNAP COPY-buffer tuning, external-unmount teardown or
broad read-path optimization only for a concrete new correctness gap or
measurement. Longer-term proposals live in
[FOD_FUTURE_IDEAS.md](FOD_FUTURE_IDEAS.md), not in this queue.

## Completed work and supporting reviews

The former plan, including completed fallocate, sparse seek, create/rename,
open/unlink convergence and inode/path cache retirement, is preserved in
[the 2026-10-06 plan archive](../history/FOD_CURRENT_PLAN_2026-10-06.md).
[HISTORY.md](../HISTORY.md) indexes older completed plans and measurements.
The [documentation index](../README.md#correctness-reviews-and-evidence)
connects the transaction, privilege, write-fencing and authority reviews.

Update this queue when a task closes or its priority changes. Keep detailed
experiments and acceptance matrices in the linked topic document rather than
duplicating them here.
