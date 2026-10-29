# FOD documentation

Documentation is organized by task first, then by lifecycle class. Current task guides stay directly under `docs/`; historical evidence lives under [`history/`](history/); active and future plans live under [`plans/`](plans/).

## Documentation classes and source-of-truth order

| Class | Use it for | Primary locations |
| --- | --- | --- |
| Current behavior | What FOD does now and how to operate it | [`CURRENT_STATE.md`](CURRENT_STATE.md) and the task guides below |
| Durable contracts | Compatibility, security, schema/storage and architecture decisions expected to outlive one release | requirement/contract documents and [`adr/`](adr/) |
| Historical evidence | Release-specific implementation notes, measurements, compatibility/toolchain baselines and completed project records | [`HISTORY.md`](HISTORY.md) and [`history/`](history/) |
| Plans | One implementation queue, detailed workstreams, and future proposals | [`plans/CURRENT.md`](plans/CURRENT.md), [`plans/`](plans/), [`../ROADMAP.md`](../ROADMAP.md) |
| Reviews | Dated findings and evidence supporting current work; not independent queues | Correctness reviews listed below |
| Work journals | Execution logs and accumulated conclusions retained for traceability | `../commands.md`, `../conclusions.md` |

When documents disagree about **current behavior**, use this order:

1. current code, tests, configuration and authoritative version metadata,
2. [`CURRENT_STATE.md`](CURRENT_STATE.md) and the current task guide for the area,
3. durable requirement/contract/ADR documents,
4. historical evidence and work journals,
5. plans and proposals.

Architecture and security requirements remain normative. A code/contract
mismatch is a defect or an explicit design decision to resolve; the order above
does not authorize weakening a contract to match an implementation.

Historical evidence is intentionally not rewritten to match later defaults.
Update the current task documentation instead.

## Reading path and document ownership

Read [CURRENT_STATE.md](CURRENT_STATE.md) for implemented behavior, then
[architecture invariants](FOD_ARCHITECTURE_INVARIANTS.md) for required guarantees.
Use [plans/CURRENT.md](plans/CURRENT.md) to select work and follow its topic links
for detailed design, evidence and acceptance criteria.

| Information | Single owning document | Other documents should contain |
| --- | --- | --- |
| Current capabilities and limitations | [CURRENT_STATE.md](CURRENT_STATE.md) | A link and topic-specific details |
| Shared correctness requirements | [FOD_ARCHITECTURE_INVARIANTS.md](FOD_ARCHITECTURE_INVARIANTS.md) | References to the applicable invariant |
| Implementation order and closure status | [plans/CURRENT.md](plans/CURRENT.md) | Evidence or detailed acceptance criteria, without a competing queue |
| HA/recovery phases and cache matrix | [HA/cache workstream](plans/HA_READONLY_RECOVERY_AND_CACHE.md) | Links to this workstream and its contract review |
| Future direction | [ROADMAP.md](../ROADMAP.md), [future ideas](plans/FOD_FUTURE_IDEAS.md) | Proposals, without presenting them as implemented |
| Completed work and measurements | [HISTORY.md](HISTORY.md), [history/](history/) | Dated evidence and links to the current owner |

When updating documentation, revise its existing owner first. Add a separate
review only when it contributes distinct findings or evidence; link it from
this index and the relevant workstream. Move completed plans to `history/`,
update current links, and retain a short pointer where old paths need to work.
`TODO.md`, `commands.md` and `conclusions.md` retain decisions and chronological
records; they do not define the current implementation order.

## Correctness reviews and evidence

These documents support the queue in [plans/CURRENT.md](plans/CURRENT.md).
Their dated findings are evidence; they do not by themselves certify that every
reported gap is still present or that a proposed fix has passed acceptance.

| Area | Contract/design | Review and evidence |
| --- | --- | --- |
| PostgreSQL authority and mutation ordering | [Architecture invariants](FOD_ARCHITECTURE_INVARIANTS.md) | [Primary authority audit](FOD_PRIMARY_AUTHORITY_AUDIT.md), [C6 write fencing](FOD_C6_WRITE_FENCING_AUDIT.md) |
| Filesystem API compatibility | [POSIX.1-2024 conformance scope](POSIX_CONFORMANCE_AUDIT.md) | [Errno reference](ERRNO_REFERENCE.md), [verified FUSE compatibility contracts](compatibility-contracts.md) |
| Cross-mount content coherence | [TX03J contract and stages](TX03J_CROSS_MOUNT_CACHE_COHERENCE.md) | [Generation coverage and remaining gates](tx03j-generation-audit.md) |
| Standby availability and recovery | [HA/cache plan](plans/HA_READONLY_RECOVERY_AND_CACHE.md) | [Degraded read-only contract review](FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md) |
| Transaction boundaries and privileges | [Security](SECURITY.md), [runtime privilege policy](FOD_RUNTIME_PRIVILEGE_POLICY.md) | [C10 privilege audit](FOD_C10_POSTGRES_PRIVILEGE_AUDIT.md), [C10-B SQL inventory](FOD_C10B_SQL_MUTATION_INVENTORY.md), [C10-B2 transaction review](FOD_C10B2_HIGH_RISK_TRANSACTION_REVIEW.md) |
| Payload ownership | [Space accounting](space-accounting.md), [storage-format ADR](adr/storage-format-versioning.md) | [Payload ownership inventory](storage-payload-ownership-inventory.md) |

## 1. Understand the current system

- [`CURRENT_STATE.md`](CURRENT_STATE.md) - compact current architecture, deployment model, I/O size layers and lifecycle guarantees.
- [`../README.md`](../README.md) / [`../README.pl`](../README.pl) - project entry pages.
- [`../ROADMAP.md`](../ROADMAP.md) - current direction.
- [`../TODO.md`](../TODO.md) - historical decisions and follow-ups; the active queue is [`plans/CURRENT.md`](plans/CURRENT.md).

## 2. Install or deploy FOD

- [`DOCKER_DEPLOYMENT.md`](DOCKER_DEPLOYMENT.md) - complete PostgreSQL + FOD Docker deployment.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FOD/FUSE client layer.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - persistent host startup and systemd integration.
- [`OPERATIONS.md`](OPERATIONS.md) - day-2 status, upgrade, restart and reboot procedures.
- [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md) - native Ubuntu/Debian and RHEL/Rocky package model.

## 3. Configure runtime, PostgreSQL and FUSE

- [`runtime-configuration.md`](runtime-configuration.md) - configuration precedence and runtime controls.
- [`runtime-env-ini-audit.md`](runtime-env-ini-audit.md) - generated environment/INI audit.
- [`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) - PostgreSQL requirements and tuning boundaries.
- [`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md) - PostgreSQL session contract.
- [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md) - FUSE/kernel capabilities and request-size constraints.
- [`FOD_STORAGE_BLOCK_SIZE_SELECTION.md`](FOD_STORAGE_BLOCK_SIZE_SELECTION.md) - storage-block design background.

Current high-level defaults are summarized in [`CURRENT_STATE.md`](CURRENT_STATE.md). Historical sizing experiments are indexed by [`HISTORY.md`](HISTORY.md).

## 4. Security, permissions and host policy

- [`SECURITY.md`](SECURITY.md) - security/permissions entry point.
- [`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md) - authoritative root-operated runtime policy.
- [`history/FOD_3_3_22_ROCKY_SELINUX.md`](history/FOD_3_3_22_ROCKY_SELINUX.md) - verified Rocky Linux 10.2 SELinux evidence.
- [`LOCAL_TEST_DATA_PRIVACY.md`](LOCAL_TEST_DATA_PRIVACY.md) - local/private test-data handling.

## 5. Profile or optimize performance

- [`performance.md`](performance.md) - canonical profiling workflow and Make targets.
- [`../BENCHMARKS.md`](../BENCHMARKS.md) - accepted benchmark baselines.
- [`performance-baselines.md`](performance-baselines.md) - baseline details.
- [`FOD_RANDOM_IO_POSTGRESQL_TUNING.md`](FOD_RANDOM_IO_POSTGRESQL_TUNING.md) - PostgreSQL random-I/O tuning background.
- [`plans/CURRENT.md`](plans/CURRENT.md) - single queue for correctness and performance work.
- [`HISTORY.md`](HISTORY.md) - dated, release-specific and completed performance-plan evidence.

## 6. Index or import external sources

- [`fod-indexer.md`](fod-indexer.md) - complete indexer guide.
- [`fod-indexer-read-api.md`](fod-indexer-read-api.md) - read-only integration contract.

## 7. Test or validate a change

- [`../zasady_sprawdzen.md`](../zasady_sprawdzen.md) - step-by-step validation profiles and ordering rules.
- [`../README.md`](../README.md#quality-gates) - minimum release/policy gates.
- [`performance.md`](performance.md) - profiling gates for performance work.

The repository intentionally has no active GitHub Actions workflow. Validation uses local Make/Cargo gates.

## 8. Develop FOD, change schema or compatibility contracts

- [`DEVELOPMENT.md`](DEVELOPMENT.md) - development entry point.
- [`versioning.md`](versioning.md) - one-version-per-commit rule.
- [`compatibility-contracts.md`](compatibility-contracts.md) - compatibility contracts.
- [`space-accounting.md`](space-accounting.md) - capacity/accounting contract.
- [`MAKE_TARGET_NAMING.md`](MAKE_TARGET_NAMING.md) - Make target naming policy.
- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md) - storage-format versioning ADR.
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md) - storage object/segment manifest ADR.

## 9. Historical evidence

- [`HISTORY.md`](HISTORY.md) - categorized evidence map.
- [`history/README.md`](history/README.md) - physical history-directory policy.
- `../commands.md` - chronological execution journal.
- `../conclusions.md` - accumulated implementation/benchmark conclusions.

Historical files remain as recorded and are not canonical sources for later defaults.

## 10. Plans and future work

- [`plans/README.md`](plans/README.md) - plans index and lifecycle rule.
- [`plans/CURRENT.md`](plans/CURRENT.md) - the single current implementation queue.
- [HA/cache workstream](plans/HA_READONLY_RECOVERY_AND_CACHE.md) - detailed recovery phases and performance acceptance.
- [`plans/FOD_FUTURE_IDEAS.md`](plans/FOD_FUTURE_IDEAS.md) - longer-term ideas and proposals.
- [`../ROADMAP.md`](../ROADMAP.md) - long-term roadmap.
- [`../TODO.md`](../TODO.md) - historical follow-up/archive record; it is not the active queue.

Completed plans and old execution roadmaps are retained under [`history/`](history/) rather than mixed with active/future plans.
