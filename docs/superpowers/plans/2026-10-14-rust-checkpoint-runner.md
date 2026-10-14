# Native checkpoint and measured runner implementation plan

> **For agentic workers:** Use superpowers:executing-plans inline. One fresh independent final review and one verified Important/Critical correction pass; no rereview. User explicitly authorized selfapproval and continuing to the ten-week objective.

**Goal:** Самостоятельно считать X недель отдельного текущего мира в RAM, сохранять/восстанавливать весь мир и обязательную историю, фактически выполнить10 недель и дать достоверный диагноз.

**Architecture:** Streaming atomic checkpoint с самостоятельным SourceSnapshot; CLI оркестрирует существующие закрытые недели, компактные метрики и сравнение продолжений. Рабочая SQLite/runtime readonly; никаких экономических подмен.

**Tech Stack:** Existing serde/json, sha2, rusqlite, std filesystem/thread/process; no new external service or compatibility system.

**Spec:** `docs/superpowers/specs/2026-10-14-rust-checkpoint-runner-design.md`

## Global constraints

- Frozen rules/source during a measured series; one raw per province, actual recipes, taxes, rights and capital; no cash/goods/orders seeding.
- Calendar12×30, +7days/one revision; workers1, RAM; separate save/load timing, no SQL/world export each week.
- Native SourceArc/raw unknown author fields and typed IDs must survive save/load. New output directory only; no working campaign/DB/runtime changes.
- Fullworld0/10 before this stage; mini tests are not fullworld proof. Missing metrics explicit, whole property audit limits stated.
- No Git repository: before snapshot/manifest and ledger, not invented commit IDs. Preserve diagnostic evidence rather than delete it on finish.

## Review focus

1. Full lossless unknown-source retention: dictionary/shared tuple keys, source raw payload/ordinal/sequence and typed owner/currency IDs.
2. Interrupted write/load corruption or path tricks cannot replace a successful snapshot or read arbitrary manifest paths.
3. Replayed/cancelled weeks never duplicate money/goods/date; new-process historical receipts remain valid after later legitimate cohort transitions.
4. Metrics derive from genuine operations/paid supply rather than potential capacity or zeros; state/province/class aggregation reconciles with world.
5. Current-full-world input audit and isolated output preserve working source; continuous/restored comparison includes mandatory history but excludes only technical diagnostics.

## Task1: Streaming independent checkpoint

Files: new `crates/kingmaker-io/src/checkpoint.rs`, io lib/Cargo.toml; tests `checkpoint_test.rs`.
Interfaces: `save_checkpoint(&WorldState,&Path,&CheckpointIdentity) -> Result<CheckpointManifest>`, `load_checkpoint(&Path) -> Result<(WorldState,CheckpointManifest)>`. Fixed files/schema1, source rows streamed as typed JSONL, stream SHA256/size. Account index rebuilt; no engine behavior change.

- [x] RED self-contained source+native roundtrip with unknown/raw rows, distinct string/number IDs, shared tuple keys, no Source and genuine mandatory week history; source DB unavailable after save.
- [x] RED existing target remains unchanged, partial/missing/truncated/altered files and wrong schema/header/hash refuse; malformed account address refused, fixed manifest file names.
- [x] Implement borrowed streaming records and atomic temporary-directory publication. GREEN affected tests; no fullworld claim.

## Task2: Initial-world audit and compact economics

Files: new sim `world_audit.rs`, `week_metrics.rs`; tests for each using existing genuine mini worlds.
Interfaces: read-only `audit_initial_world(&WorldState) -> InitialAudit`; `collect_week_metrics(beforeSummary,&WorldState,&NativeWeekReceipt) -> WeekMetrics`; summary frozen without copying full world.

- [x] RED invalid resource/recipe/staffing, duplicate labor, missing owner/reference, negative/nonfinite values and rights failures produce specific ID/category; valid partial economics permitted.
- [x] RED real production/consumption/money/profit/taxes/construction/investor/supply metrics reconcile by world/state/province/class; unsupported fields explicit. First anomaly week retained.
- [x] Implement from actual authoritative state/receipts/operations; avoid repeated whole graph scans per order. GREEN.

## Task3: Autonomous cancellable CLI

Files: new CLI runner/progress modules; existing main; native_week cancellable adapter, readiness metadata; CLI integration and native cancellation tests.
Interfaces: explicit `--run-weeks X --output DIR`, `--resume CHECKPOINT`, `--verify-continuation CHECKPOINT --output DIR` after inspecting actual config contracts. Existing inspect read-only and legacy detached simulation refused. Internal `try_prepare_native_week` progress callback returns OpResult without weakening current public wrapper.

- [x] RED real multiweek mini CLI in one process, X/date/revision, separate output/source unchanged, readable N/X timing; bad args never write/load unnecessary world.
- [x] RED STOP between phases leaves current week unpublished, saves last complete world; failed phase stops that series with reproducible request. Heartbeat shows active phase elapsed time with no model call.
- [x] Implement RAM loop, initial/end/cancel checkpoints and compact JSONL/CSV. GREEN. Fullworld remains guarded until checkpoint/CLI validations pass.

## Task4: Fresh-process restoration and equality

Files: checkpoint CLI continuation/economic comparison helper and integration tests. Compare streaming canonical economic/native required history, excluding only recorded technical timings/diagnostics; source/raw rows also compared. No additional expensive full digest.

- [x] RED genuine full mini week saved, new process loaded, next week agrees with continuous calculation; old request replays without effects, altered mandatory history refuses.
- [x] GREEN multiple currencies/typed IDs, construction/capital receipts, dated shipment continuation and calendar rollover. Keep save/load/run timings distinct.

## Task5: Final correctness gate and current EXE

- [x] Fresh affected/workspace tests, release checks/build; retain excluded unchanged costly Source loader proof.
- [x] ONE fresh final review; regrade and fix verified Important/Critical RED→GREEN once, minors recorded, no rereview.
- [x] Publish verified CLI/server with truthful per-entry capabilities; freeze measured engine/rules/source manifest. Selfapproved artifacts/decisions/results recorded.

## Task6: Actual full-world10 weeks and report

- [ ] Read current runtime readonly; freeze campaign/source/revision/date/fingerprints/workers/RAM/automatic vs player controls; initial fullworld audit. Save independent diagnostic copy and verify new-process load.
- [ ] Run10 actual consecutive autonomous weeks in RAM, compact progress/results. Significant bug: stop only invalid series, reproduce/regress/fix engine, freeze new manifest before restarting; no balance/data tuning to make metrics good.
- [ ] Save final snapshot, load separate process and compare one extra continuation week with continuous branch. Missing/incomplete checks explicit.
- [ ] Report start→finish, graphs, world/state/problem province/goods/POP detail, first week/ID/causal chains, five categories and priorities; separately unactivated/absent mechanics. Natural construction inactivity investigated or separate mini experiment.
- [ ] Mark the overall goal complete only after all required actual measurements/restoration/report are finished; answer four user economics questions factually.

Selfreview: Task1 preserves raw/source contracts, Task2 separates initial-data defects and actual economics, Task3 autonomous execution/cancellation, Task4 restoration, Task5 independent gate/frozen executable, Task6 actual objective. This written plan is selfapproved under standing user permission; no new product edits before the plan. Stage10 review correction/publication must finish first.
