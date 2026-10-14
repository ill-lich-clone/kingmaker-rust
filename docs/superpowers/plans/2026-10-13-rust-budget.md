# Rust Budget Opening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans; inline execution and one final fresh independent review.

**Goal:** Реальное атомарное денежное открытие будущей полной недели, без новой балансовой нормы.

**Architecture:** Lossless BudgetData projection → OperationSession private budget authorization → закрытый PreparedBudget/read set → единая публикация и replay receipt.

**Tech Stack:** Rust stable, существующие serde/rusqlite/точные money_math; JS oracle только на небольших тестовых фикстурах.

**Spec:** docs/superpowers/specs/2026-10-13-rust-budget-design.md.

## Global Constraints

- Автор разрешил Codex самостоятельно утвердить спеку и план; утверждено 2026-10-13, выполнять здесь.
- Фиксированные authored rates, reserve_weeks=2, funding 0–100, FTE и права сохраняются; никаких подгонок стартового мира.
- Не запускать полноценные недели до реализации всех обязательных фаз. Рабочая DB/runtime.json прежние; источник только для чтения.
- Git отсутствует: проверенная резервная копия исходников вместо коммитов; сохранять единственную восстановимую копию.

## Review Focus

1. Пересечение служебных и production assignments: нельзя платить дважды (Task 3).
2. Нулевой ползунок и старый долг: погашается долг, не создаётся долг по отменённому запросу (Task 3).
3. Нативная эмиссия не разрешает внешний поддельный DTO (Task 2).
4. Изменение работодателя/ставки/профессии после подготовки требует отказа без частичных изменений (Task 4).
5. Числовые и строковые ID, несколько валют, rounding remainder не смешиваются (Tasks 1–3).

### Task 1: Данные и oracle

**Files:** core/src/budget.rs, core/src/state.rs, io/src/budget_projection.rs, io/src/projection.rs, io/tests/budget_projection_test.rs; scripts/tests/budget-oracle.cjs.

**Interfaces:** BudgetData { employment, jobs, adventurer_income_rates, foundation_budget_policy, phase_receipts }; BudgetRequest { expected_revision, period, policies }; lossless Extra.

- [x] RED: projection fixture сохраняет typed IDs/extra/null и отказывает на неоднозначных singleton.
- [x] Реализовать project_budget(source, codec) -> Result<BudgetData>, подключить к loader.
- [x] GREEN: cargo test -p kingmaker-io --test budget_projection_test.
- [x] Сохранить маленький JS oracle opening/service/requirements с BigInt→строки, без SQL и source DB.

### Task 2: Авторизованная эмиссия и финансирование

**Files:** sim/src/budget_funding.rs, sim/src/budget_cycle.rs, sim/src/operation_session.rs, core/src/operations.rs, sim/tests/budget_funding_test.rs.

**Interfaces:** private stage_budget_operation(session, op); plan_adventurers/session, plan_state_funding/session, plan_private_funding/session. Экономические операции AdventurerIncome/StateBudgetFunding/BudgetFunding принимают только точную приватную авторизацию; баланс изменяется только на подтверждённую эмиссию в её валюте.

- [x] RED: разрешённый доход 10×200=2000, net=1800, tax=200; поддельные прямые операции отвергаются; конечный бюджет и раздельные сектора/валюты; два источника частного резерва.
- [x] Реализовать существующие формулы и точные event IDs, индексы по контрактам/ПОПам.
- [x] GREEN: budget_funding_test и прежние operation_session/market/production suites.

### Task 3: Реальная служебная зарплата и старые долги

**Files:** sim/src/service_payroll.rs, sim/src/production_payroll.rs, sim/tests/service_payroll_test.rs.

**Interfaces:** plan_service_payroll(session, request) -> OpResult<Vec<EconomicOperation>>; reuse plan_prior_production_obligations.

- [x] RED: 0/50/100% → gross 0/500/1000; treasury 1000 → по 500 двум секторам и общий unpaid debt 1000; старый долг при funding=0; strict initial_plan; slaves; production/direct_output exclusions; двойной труд/превышение мест отвергается.
- [x] Реализовать эффективный труд + реальные ставки/политики, авторизовать внутреннюю зарплату, сохранять бюджетные поля в новых и оплаченных долгах. Не менять отдельно существующее правило production gross=0.
- [x] GREEN: service_payroll_test + JS differential oracle.

### Task 4: Атомарность, replay, сохранность и публикация

**Files:** sim/src/budget_cycle.rs, sim/src/budget_invariants.rs, sim/tests/budget_cycle_test.rs; reports/rust-budget-2026-10-13/*.

**Interfaces:** prepare_budget_cycle(&WorldState,&BudgetRequest)->OpResult<PreparedBudget>; apply_budget_cycle(&mut WorldState,PreparedBudget)->OpResult<BudgetPhaseReceipt>; execute_budget_cycle wrapper.

- [x] RED: полный цикл сравнить с JS, ранняя ошибка не оставляет выплаты, повтор не платит, конфликт истории/изменённый read set отказывает, денежный рост объясняет только утверждённая эмиссия, люди/товары/труд прежние.
- [x] Реализовать закрытый read set и receipt, явные суммы эмиссии/налогов/долгов; не использовать полный WorldState clone на операцию.
- [x] GREEN: affected suites; один итоговый свежий review; TDD исправить Important/Critical одним проходом; build release/publish текущие EXE; записать отчет и продолжить следующий этап без нового запроса утверждения.
