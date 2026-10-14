# Native investment and construction implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans inline; one final fresh independent review. Steps use checkboxes.

**Goal:** Реальные совместные инвестиции, оплаченная стройка и сохранённые имущественные права в Rust без изменения согласованных правил.
**Architecture:** Existing WorldState/OperationSession and catalogs remain authoritative. Lossless ConstructionData only for currently missing author registries. One private phase state; canonical history/evidence/readsets. Construction payroll shares native payment rules, not a second service salary.
**Tech Stack:** Existing Rust workspace, serde, exact Money/BigInt, terminal JS oracle only for miniature tests.
**Spec:** docs/superpowers/specs/2026-10-13-rust-construction-design.md, self-approved under user's standing authorization. Begin product edits after income Stage7 final gate.

## Global constraints

- No balance changes, manual world seeding, source/working campaign writes during probes, runtime data substitutions or alpha migrations.
- Eight regional factory slots, unique method/province, 40000 new-factory people_slots, level20 expansion limit; other buildings use their existing rules.
- Protected reserve, investment/refund purposes, rational net shares, exact taxes and actual paid FTE preserved.
- Source recipes/sites/terms/technology must be present and valid; unsupported diagnostics explicit.
- Full weeks remain guarded until dated Supply, history/checkpoint and assembled pipeline are ready. Goal ten actual weeks remains active.
- Backup/manifests replace Git commits here; one final review, one Critical/Important correction pass, no re-review; verified CLI/server publication at stage end.

## Review Focus

1. Shared investor account across projects/rounds cannot promise or spend the same money twice.
2. Stale source assignment halfway through staff activation leaves every worker/contract/owner unchanged.
3. Construction cash price including wages must deduct real gross paid wages once, with partial payroll and prior debt respected.
4. Multi-investor cancellation after materials/cargo leaves exact money refunds and legitimate remaining goods/cargo claims.
5. A completed factory's method/facility/operating capital and rights survive load and ordinary next opening hiring, without manual assignments.

### Task1: Missing author registries and closed operation authority

Files: core/src/construction.rs, core/src/state.rs/lib.rs/operations.rs; io/src/construction_projection.rs/projection.rs/lib.rs; io/tests/construction_projection_test.rs; sim/src/construction_operations.rs, operation_session.rs; tests construction_authority_test.rs. Paths are under crates/kingmaker-*.
Interfaces: ConstructionData projects reuse existing typed production rows; new fields investment_opportunities, construction_catalog, construction_catalog_by_currency, building_site_grants and phase_receipts. Existing capitalist_entities/project_investments remain mobility_data, not duplicates.
- [x] RED lossless numeric/string actor IDs, currencies, nested source/terms/unknown fields, author value change reflected after projection and native serde.
- [x] RED public arbitrary contribution/material/fee/refund payload rejected; private planned authority permits only exact operations.
- [x] Implement minimal projections/authority; targeted IO/core/session checks GREEN; actual full-source readonly inspect only if projection scope changed.

### Task2: Assessment, proposal, contributions and activation

Files: sim/src/investment_assessment.rs, investments.rs, construction_limits.rs, tests/investments_test.rs, scripts/tests/construction-oracle.cjs.
Interfaces: readonly assess_opportunities; prepare/apply/execute_automatic_investments(world, ConstructionRequest(expected_revision,period,policies)); receipt operations/decisions/blocked/registry/evidence. Authored opportunity is input, computed conditions authoritative.
- [x] RED source ordinary decisions/terms/site/technology/margin/input/workforce/currency/slot gates and first viable opportunity order.
- [x] RED two investors/tranches/shared account, min cash-reserve-limit-budget, protected marginal roundRatio, exact rational shares and zero remaining, conflicting/duplicate receipt and stale readset.
- [x] RED actual funded staff activation with stale later transfer, no partial movement/double labor; preserve contracts and capital entities.
- [x] Implement using native quotes/market scope/labor/finance helpers; targeted GREEN and atomic mini serde replay.

### Task3: Construction labor and real payroll

Files: sim/src/construction_labor.rs/payroll.rs, production_payroll.rs as shared helper requires, tests/construction_labor_test.rs.
Interfaces: plan_construction_labor from actual projects/stock/contracts; closed payroll receipt consumed by construction, prerequisite exact history; no caller-supplied paidLabor bypass.
- [x] RED source materials unavailable, wrong pop/province/contract, remaining FTE-days, scarce escrow and partial exact gross/tax/net, includes/excludes wage price, zero wages or prior obligation.
- [x] RED existing service/producer payroll never pays same construction employment twice; explained actual FTE only.
- [x] Implement minimal shared exact payroll/session authority, focused GREEN, affected budget/production/labor regressions.

### Task4: Progress, completion and expansion

Files: sim/src/construction.rs, construction_rights.rs, construction_cycle.rs/invariants.rs, tests/construction_cycle_test.rs.
Interfaces: prepare/apply/execute_construction(world,request) validates canonical actual payroll; project/readset/operations/receipt publication atomically; stage boundaries required for empty phases.
- [x] RED materials consumed once, paidFTE elapsed progress, material/labor/fee/reserve/config wait cases and no free completion.
- [x] RED real mini fully financed factory, actual fee/working capital/refunds/net shares; ordinary next opening can hire/produce; mini serde continuation/replay.
- [x] RED expanded old factory consent/base-capital stale, level cap, rational combined rights, equipment/storage/slots; RGO and other building limits/effects.
- [x] Implement source formulas; money/goods/people/labor/rights deltas checked; affected GREEN.

### Task5: Cancellation and required history

Files: sim/src/construction_cancellation.rs; tests/construction_cancel_test.rs, IO/native serde checks.
Interfaces: closed cancellation request event/project/reason; actual source-purpose refunds and immutable goods/cargo claims, no external shipping executor imitation.
- [x] RED cancellation partial/full financing, already spent wages/materials, original purposes, rounding, excess money/no rights, conflicting repeats/completed denial; remaining stock/cargo claims retain investors.
- [x] RED missing/tampered history/receipt/registry, lawful subsequent state replay, stale readset, no mutation after late failure.
- [x] Implement and verify targeted + `cargo test --workspace -- --skip test_loads_2679_provinces_from_sqlite` GREEN.

### Task6: Final review, publication and next stage

- [x] One fresh review against verified before manifest, spec/plan/ledger and all five review focus inputs; verified Important/Critical RED→GREEN in one pass, Minor ledger only.
- [x] Release affected checks/build; verified CLI/server backups/hashes and stage report/results. No full-world week claim; proceed dated Supply/food/state buy → pipeline/checkpoint/runner → actual10weeks and factual economy diagnosis.

Preflight: shared registries have one owner; project payroll prerequisite canonical evidence; investment precedes production/construction; no simulation can bypass full-week guard. Self-review mapped observable behavior, failure/partial/replay branches and conservation; specs/plans self-approved under standing authorization, no additional permission required.
