# Native income and artisan decisions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans inline, with one fresh final independent review. Steps use checkboxes.

**Goal:** Перенести реальные доходы, действительные права, оборотные резервы и оплачиваемую смену метода кустарей без изменения баланса.
**Architecture:** Existing WorldState/OperationSession remain authoritative. Add lossless income/transport projections, indexed native plans and opaque prepared changes; reuse money, market, labor, storage and calendar helpers. One private phase state, not one full copy per enterprise.
**Tech stack:** Existing Rust workspace, serde, exact Money helpers; JS only in small terminal test oracle.
**Spec:** docs/superpowers/specs/2026-10-13-rust-income-design.md, self-approved under user's standing authorization.

## Global constraints

- Actual authored formulas, rational ownership, 2 weekly wage reserve, authored 25-day factory buffer, quoted artisan capital forecast and taxes preserved; no new quantity/cash/balance rules.
- Existing mutable rights in mobility_data reused. No new duplicate accounts, manual seeding, migrations or raw SourceSnapshot fallback after changes.
- Full weeks blocked until construction, physical supply and checkpoints are integrated. Full source is readonly; diagnostic RAM campaign only.
- Atomic canonical operations, meaningful RED→GREEN tests and mandatory history/replay; exact IDs and unknown metadata survive serde.
- No Git here: verified source backup and manifests instead of invented commits; user already chose inline execution/self-approvals/current EXE publication.

## Review focus

1. Shared holding owns both RGO and factories: distinguish actual positive sector profits and rights without duplicating cash/reserves (Task4).
2. Two artisans share owner/input/fleet and scarce cash: reserve/forecast must not promise the same resource twice (Task2/5).
3. Initial-stock sales, freight and input consumption: recognized taxable income must have exact operation basis, preserving loss carry (Task3/4).
4. Prepared plan after reference/price/policy/order/history mutation: reject before any debit or metadata write (Task6).
5. Pending retooling across serde/date change: pay provider once, keep old method until due and preserve technologies and all unknown terms (Task5/6).

### Task 1: Lossless native state and opening basis

Files: create core/src/income.rs, core/src/transport.rs, io/src/income_projection.rs, io/tests/income_projection_test.rs; modify core/src/lib.rs, state.rs, io/src/lib.rs, projection.rs and CLI/src/main.rs readonly diagnostic counts; create sim/src/income_basis.rs, tests/income_basis_test.rs.
Interfaces: WorldState.income_data and transport_data; IncomeOpeningBasis is captured from current native world with campaign/revision and necessary stock metadata, opaque to public payload construction. Producer-income rows are mutable; transport rows have one authoritative location. Shipment rows remain existing mobility_data.shipments.
- [x] RED source singleton/rows projection preserves producerIncome, transportAssets/transportTerminals/supplyRoutes/routeReservations/navigationGeography/automatic_land_paths and every referenced route permission/catalog; numeric/string IDs, unknown fields, authored changes and serde. Audited routeReason uses these rows plus existing market territory/sale offers; automatic findLandPath uses existing territory/native graph and per-phase cache, no additional financial permission table. Actual readonly full source inspection passed.
- [x] Run `cargo test -p kingmaker-io --test income_projection_test` → expected missing API/fields, then implement minimal lossless projection. Run again → green.
- [x] RED opening basis preserves start metadata under current-stock changes, rejects wrong campaign/revision or capture after phases; implement and verify `cargo test -p kingmaker-sim --test income_basis_test`.

### Task 2: Native working-capital and transport forecast

Files: create sim/src/income_capital.rs, transport_validation.rs, tests/income_capital_test.rs, scripts/tests/income-oracle.cjs; modify sim/src/lib.rs as required.
Interfaces: phase-indexed CapitalForecast returns entities/groups with exact minimum/requested/possible/output; readonly forecast never transfers money, goods or rights. Group by exact owner/currency; local scope reuses current regional resolver, remote routes retain travel/freight/permissions.
- [x] RED compare actual JS workingCapitalRequirements for own/held/food stocks, next-7-day inbound, blocked border/terminal, fleet busy until release, shared input reservations, two enterprises/one owner, scarce personal+operating money, pending next method and authored minimum. No invented available=0 for unsupported routes.
- [x] Run `cargo test -p kingmaker-sim --test income_capital_test` → expected RED; implement indexed source algorithms using existing price/money/calendar helpers; run → green. Preserve all source group fields needed by income/choices.

### Task 3: Recognition and private payment authority

Files: create sim/src/income_recognition.rs, income_operations.rs, operation_history.rs, tests/income_recognition_test.rs; modify core/src/operations.rs, production.rs, sim/src/operation_session.rs, production.rs and tests/production_cycle_test.rs. Compact per-operation SHA evidence closes the reproduced production replay history gap before that history can justify income.
Interfaces: recognized rows derive only canonical committed required phase operations + opaque native basis. New enterprise_income/dividend/retooling kinds are authorized only by internal exact plans. Money legs conserve currency; no public event payload authorizes income issuance.
- [x] RED initial vs produced sale, mixed-stock price allocation, actual consumed inputs, wages/rent/maintenance/freight/recovery, old-period history exclusion, forged/missing/disguised required event and arithmetic overflow. Pin JS formulas and op identity.
- [x] Run recognition/session tests → RED; implement recognition and guarded session operations; run → GREEN, retaining arbitrary-payload rejection tests.

### Task 4: Producer profit, shares and dividends

Files: create sim/src/producer_income.rs, producer_reserves.rs, profit_rights.rs, tests/producer_income_test.rs.
Interfaces: exact recognized income + current financial/rights dependencies → producerIncome rows and closed dividend operations; no direct WorldState mutation from public operation input.
- [x] RED prior loss/net/basis, no-profit, initial-stock cash alone, unresolved quotes, 2w wage/sponsored maintenance/25d input/debt reserve; RGO/factory mixed holdings, approved authored shares and project-investor fallback; rational sum/last remainder/taxes/state share/multiple currencies/missing beneficiary.
- [x] Run producer-income tests → RED; implement source algorithms, inherited policy merging and exact reserve math; run → GREEN and money/title invariant checks.

### Task 5: Artisan payouts, quotes and paid retooling

Files: create sim/src/artisan_income.rs, artisan_choices.rs, tests/artisan_income_test.rs, artisan_choices_test.rs.
Interfaces: income updates existing ArtisanEnterprise extra/state/working_capital; opening completes only due pending method, closing prepares paid choice using actual unsatisfied orders/quotes/capital forecast. Reuse calendar and exact existing stock access rules.
- [x] RED revenue/initial stock/cost/loss/principal/retained profit, authored-vs-shared-capital reserve, FTE weighted distribution and legitimate owner property income, net+tax conservation, empty/wrong/missing assignments/POP currency; compare JS oracle.
- [x] RED automatic_choice=false, no technology/raw/demand/margin/cash, cost provider, tie sorting, 64-step feasible output, same method, pending/due dates, completion and no second charge across roundtrip. Compare real JS choices/retooling/quotes functions.
- [x] Run focused artisan tests → RED; implement exact source rules and closed opening/closing plans; run → GREEN, without runtime author replacements.

### Task 6: Atomic closing and required history

Binding interface extension from Task5 stock provenance: publish engine-derived producer quantities atomically, preserve them through actual production/consumption/cohort stock transfers and native serde, with regressions before final review. The dated Supply executor must then preserve the same provenance through cargo under its canonical phase evidence; this remains part of the full goal, not a fixture substitute.

Files: create sim/src/income_cycle.rs, income_invariants.rs, tests/income_cycle_test.rs; modify core income receipts, state view derivation and lib exports.
Interfaces: IncomeRequest(expected_revision,period,policies) plus opaque basis; prepare/apply/execute closing produces canonical phase receipt/operations/registry changes; caller cannot supply payroll/sales. Opening choices separate from closing income. Dated supply phase receipts will extend the same closed evidence dependency when execution is ported.
- [x] RED required budget/production/closing-market history, policy inheritance, stale stocks/prices/orders/rights/routes, late failed beneficiary rollback, orphan/duplicate/conflicting receipt, exact replay after lawful subsequent state, mini serde continuation preserving income/choices/claims/rights.
- [x] Implement one phase-private state, indexed plans and final authorized OperationSession before metadata publication. Monetary/group/title invariants explain all deltas; unknown diagnostics explicit. Run focused tests and `cargo test --workspace -- --skip test_loads_2679_provinces_from_sqlite` → GREEN; changed IO separately gets actual-source readonly probe.

### Task 7: Review, publication and continuation

- [x] One fresh final reviewer; reproduce Important/Critical with RED, one correction pass, ledger rulings and no re-review.
- [x] Build release/test affected suites, publish CLI/server with verified hashes/backups, save report/results/spec/plan/probe limits. Do not advance full world week during stage proof. Continue investment/construction → dated supply/food/state-buy → complete week/checkpoint/runner → actual10 weeks and economic diagnosis.

Self-review: every spec rule mapped to Tasks1–6; forecast is readonly while payouts/retooling use closed finance authority, basis is frozen native data while current mutable rights remain current, supplies share authoritative transport registry and existing shipments. Future-phase extension is explicit and does not substitute an incomplete helper for the complete goal.
