# Rust Local Labor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans; inline tasks and one final fresh independent review.

**Goal:** Производства и службы автоматически принимают местных свободных работников, по реальной вместимости, собственным ставкам, деньгам и производственной выполнимости.

**Architecture:** Одни BudgetData jobs/employment/contracts + typed ProductionData. LabourData добавляет авторские правила учреждений/профилей/видов и историю мобильности, PopRecord сохраняет Extra. Закрытый PreparedLocalLabor сочетает изменения строк и авторизованную денежную дельту, а общий roster проверяет людей/пулы до и после.

**Tech Stack:** Existing Rust/serde, native needs + money_math + production/storage, JS oracle на маленьких fixtures; новых runtime/миграций нет.

**Spec:** docs/superpowers/specs/2026-10-13-rust-local-labor-design.md.

## Global Constraints

- Самостоятельно утверждено 2026-10-13 по прямому разрешению пользователя; выполнять здесь, без новой остановки на согласование.
- Существующий динамический механизм портируется в движок; не пересобирать стартовые назначения/население и не добавлять деньги/товары/места.
- Level ×2000, исходные rgo_capacity/people_slots, две недели зарплат, собственная корзина, integer quota/search, совместные склады и исходные рецепты сохраняются.
- Полный недельный pipeline остаётся guarded до перемещений/доходов/стройки/транспорта/commit; исходная база read-only, тесты не становятся рабочей кампанией.
- Git отсутствует: verified source backup и ledger, без удаления единственной копии.

## Review Focus

1. Контракт больше не подходит ПОПу: освобождение до бюджета, долг прежний; не маскировать несоответствие (Tasks 2,6).
2. Общая казна/сырьё/склад между несколькими работодателями: одно обеспечение не поддерживает два найма (Tasks 3–5).
3. Увеличение/снижение уровня с несколькими ролями: сохранение total slots, FTE и deterministic remainder (Tasks 1–2).
4. Новый контракт не наследует чужую ставку; цены/нормы неизвестны — явный отказ (Task 3).
5. Повтор/частичная история/старая подготовка не удваивают FTE, tenure или переводы (Task 6).

### Task 1: Lossless data and dynamic capacities

**Files:** core/src/labor.rs, core/src/pop.rs, core/src/state.rs; io/src/labor_projection.rs, io/src/projection.rs; sim/src/labor_capacity.rs; io/tests/labor_projection_test.rs, sim/tests/labor_capacity_test.rs; scripts/tests/labor-oracle.cjs.

**Interfaces:** LabourData { institution_rules:Option<Value>, profiles:Vec<Value>, species:Vec<Value>, pop_mobility:Option<Value>, phase_receipts }; PopRecord.extra:Extra; derive_labor_jobs(&WorldState)->OpResult<Vec<Value>>.

- [x] RED projection: species/literacy/mobility unknown fields and typed refs retain values after load/change; no scalar copies drift. RED capacity: school level2→4000 roles, level1→2000,0→0, tie weights and no authored rule fail; RGO expansion and factory slots use native capacity functions.
- [x] Implement projection via codec, never reallocation/reseed; institution per-role/per-job Hamilton rounding with source collations; existing jobs linked to RGO/factory derive shared cap.
- [x] GREEN: cargo test -p kingmaker-io --test labor_projection_test; cargo test -p kingmaker-sim --test labor_capacity_test; oracle small capacity comparisons.

### Task 2: Shared roster, reconciliation, releases and tenure

**Files:** sim/src/labor_roster.rs, sim/src/labor_release.rs, sim/src/service_payroll.rs, sim/tests/labor_roster_test.rs, sim/tests/labor_release_test.rs.

**Interfaces:** build_labor_roster(&WorldState)->OpResult<LaborRoster>; occupied/free/byEmployment/byJob/byPool with typed IDs; reconcile_labor_rows jobs/POPs; release_employment(world,period)->OpResult<LaborRowChanges>. apply_staff_change(factory,departed,newcomers)->OpResult<FactoryEnterprise> implements `(tenure*(staff-departed))/(staff-departed+newcomers)`, validates safe integers and keeps existing worked-days helper.

- [x] RED: mirrored employment counts once; duplicate/inconsistent/missing refs/workforce/group/pool violations reject; inactive construction excluded; ended service excluded; no work in transit. School shrink releases excess; closed workplace/2 unpaid periods/wrong profession/land inaccessible releases. Active builders remain protected, debts/account amounts untouched. Factory departing/newcomers updates heads/tenure exactly.
- [x] Implement source roster order and release/resize rules; reuse common source-of-truth checks for budget validation after labor. All actual class mismatch in raw full world handled here before payroll, no data surgery.
- [x] GREEN: corresponding tests and source oracle; old production/budget suites still pass.

### Task 3: Own wages and finite competitive hiring funds

**Files:** sim/src/labor_money.rs, sim/src/labor_offers.rs, sim/tests/labor_money_test.rs, sim/tests/labor_offers_test.rs.

**Interfaces:** basic_basket_price(&WorldState,&PopRecord)->OpResult<Money> using resolve_need_quantities with actual size/workforce and source invoice; own_labor_contract(pop,offer,...)->OpResult<LaborContract>; available_labor_cash(...)->OpResult<LaborCash>; build_job_offers(...)->OpResult<OfferResult>.

- [x] RED: RGO basket monthly rounding then7/30 with100/110%; own factory115/service105/soldier120/officer135%; two different POP baskets differ, existing authored contract unchanged. Missing profile/price/coefficient blocks. Existing wages2weeks+debt subtract available; source treasury fallback; zero budget/multiplier0/multiple currencies never cross-fund.
- [x] Implement source quote/offer conditions and hashed indices. Match row order/typing, no new economic constant.
- [x] GREEN: source oracle DTO/rate/quote/available cash comparison; old needs/money suites.

### Task 4: Production feasibility forecast for incoming workers

**Files:** sim/src/labor_forecast.rs, sim/tests/labor_forecast_test.rs.

**Interfaces:** HiringForecast private mutable forecast maps for producer outputs, owner/province inputs after holds, global warehouse spaces, rental/wage cash; limit/accept/preview with reversible private trial state. No real operations/stock production issued.

- [x] RED: two factories share raw/storage/lease funds; first has work but second cannot duplicate reserves; no recipe input/free storage fails; mixed clerks/direct ratio and tenure/equipment potential precise; artisan pending_retooling/tech blocked; preview restores all changed keys including failures. Binary search partial labor reaches maximal feasible integer.
- [x] Implement existing source potential/demands/reserve/trial algorithms using native activity/storage/rgo formulas, not a new production model.
- [x] GREEN: source small forecast scenarios compare accepted workers/blocked conditions and state unchanged after preview.

### Task 5: Quotas, repeated local allocation and artisans

**Files:** sim/src/local_hiring.rs, sim/tests/local_hiring_test.rs.

**Interfaces:** plan_local_hiring(&WorldState,period,policies,optional filters)->OpResult<LaborPlan>; includes rows, internally derived funding ops, hires/blocked; plan_artisan_hiring uses same forecast and occupied labor.

- [x] RED: free local farmer hired now; partial slots; competing POPs/offers funds/rent and deterministic leftovers; multiple failed offers release quota same phase; source pass IDs; no infinite pass because every success consumes free FTE. Factory newcomers counted in tenure; new own contracts/jobs/employment mirrors derive valid row IDs. Artisans recover only existing own assignments with available inputs and warehouse, no salary emission.
- [x] Implement source quotas/passes/candidate ordering and monetary reservations; no sequential first-offer shortcut, no duplicated source money. Apply money privately between passes, only final publication externally.
- [x] GREEN: source oracle output rows/money/blocked/hires, plus no goods/people change and labor checked.

### Task 6: Atomic publication, integration evidence and report

**Files:** sim/src/labor_cycle.rs, core/src/labor.rs, sim/src/operation_session.rs, sim/tests/labor_cycle_test.rs; cli/examples/labor_probe.rs; reports/rust-local-labor-2026-10-13/*.

**Interfaces:** prepare_local_labor(&WorldState,&LaborRequest)->OpResult<PreparedLocalLabor>; apply_local_labor(&mut WorldState,PreparedLocalLabor)->OpResult<LaborPhaseReceipt>; execute_local_labor wrapper; LaborRequest expected_revision/period/policies.

- [x] RED: stale consulted raw/cash/recipe/levels/needs/prices/POP/class references reject before writes; replay/partial history conflicts reject. Capacity/release→local/artisan hiring→budget→real production uses accepted new staff in same period and preserves people/currency stocks/debts/rights. Existing source invalid professions are released, not paid or silently changed to fit jobs.
- [x] Implement closed prepared state/read set + exact internal funding authorization and invariant report; validate entire proposed rows before money commit. Refresh typed enterprise/account bindings after verified changes; dates/revisions unchanged in isolated helper.
- [x] GREEN: targeted suites, entire workspace (reuse unchanged valid full loader result), one fresh final review + one TDD Important/Critical pass. Full-world RAM probe on new diagnostic campaign ID records exact blockers, does not advance date or write source. Build/publish EXE, keep source fingerprint/report/spec/plan/rulings. Continue native profession/cohort/migration stage without new permission.
