# Rust Production — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Нативно выполнить действительный производственный цикл ДП, заводов и кустарей с оплатой предусмотренного труда, списанием собственных входов, законным конечным складом и реальным выпуском.

**Architecture:** Пять crates сохраняются. IO дополняет существующий мир нужными производственными данными; sim вычисляет активности и операции по актуальным JS-правилам. Нативная сессия готовит изменения затронутых строк и атомарно применяет проверенную фазу в RAM; SourceSnapshot остаётся исходной immutable-копией, а не текущим состоянием после операций.

**Tech Stack:** Rust 2021, существующие serde/serde_json/rusqlite/thiserror; `num-bigint` и `num-traits` только для точных промежуточных рациональных вычислений. JS oracle работает исключительно в тестах. При одобрении плана Cargo.lock фиксируется при добавлении этих зависимостей; затем проверки выполняются с `--locked`.

**Spec:** `docs/superpowers/specs/2026-10-12-rust-production-design.md`, одобрена автором 12 октября 2026.

## Global Constraints

- Производство строится из актуальных игровых данных, без вручную заданной цели выпуска/работников вместо состояния и без правки стартового мира для обхода отсутствующего движка.
- Источник правил — foundation-цепочка JS, а не прежний `production.cjs`/`productionPlan` или Rust-заглушка. Баланс, рецепты, зарплаты, налоги, мощности, технологии, аренда, пределы и гарантии не меняются.
- `transfer_once` сохраняет свои существующие строгие контракты. Новый credit разрешает создание только действительного счёта зарплаты/налога/арендного возврата, не неизвестного произвольного получателя.
- Все внешние ID сохраняют тип. Только номер провинции канонизируется; название холдинга/stock ID никогда не задаёт ресурс или место.
- Money(i64) хранится точно; промежуточные вычисления рациональны, непредставимый результат — явная ошибка до записи. Товарные количества/FTE и допуски повторяют JS.
- Полная неделя остаётся заблокированной. Не исполнять на полном мире даже отдельную производственную фазу. Active SQLite/runtime только читать; без миграций или переключения кампании.
- Нет нативной покупки/потребления/логистики/общего бюджета/автонайма/строительства/сохранения на этом этапе; их отсутствие не заменяется ручной подготовкой активной базы.
- Git отсутствует: сделать recoverable копию перед product-правками в `work/rust-production-2026-10-12/before`; по задачам фиксировать проверки в progress.md вместо выдуманных коммитов.

## Review Focus

1. Первое зачисление работнику без личного счёта: законный payroll создаёт только нужный purpose-счёт; отрицательный/неизвестный обычный перевод по-прежнему отвергается. Тесты задачи 3.
2. Работник распределён между двумя производствами или стройкой: FTE и время worker group не используются дважды; неизвестный коэффициент не становится единицей. Тесты задачи 5.
3. Один output goods в двух складах/у двух владельцев: входы и выход не попадают в чужой stock, единицы/held goods/held space не смешиваются. Тесты задач 4 и 7.
4. Договорная ставка/множитель 0 и частичный бюджет: учитываются committed receipts, idle guarantees, исходный налог, остаток долга и рациональное округление. Тесты задач 2 и 6.
5. Последняя операция падает или запрос повторяется после изменения балансов: откат всей фазы, совпадающий repeat до проверки новых средств, изменённые policies под тем же ключом — конфликт. Тесты задач 3 и 7.

## File Map and Interfaces

Core: `production.rs` — каталог/производители/назначения и результаты; `storage.rs` — facilities/leases/goods и space reservations; `operations.rs` — финансовые legs, obligations, receipts и prepared changes. Они определяют конкретные игровые структуры, без универсальной системы произвольных patch/plugin/миграций.

IO: `production_projection.rs` декодирует настоящие registry-поля и top-level reservations/obligations через существующий CodecContext. Неизвестные поля остаются в SourceSnapshot; authored extras изменяемой строки сохраняются при её обновлении.

Sim: `money_math.rs` — точная арифметика; `operation_session.rs` — подготовка/применение операций; `storage.rs` — реальные складские переходы; `production_activities.rs` — кандидаты; `production_feasibility.rs` — общий план; `production_payroll.rs` — действительная оплата; существующий `production.rs` — весь цикл и фактический выпуск. Каждый модуль имеет собственные граничные тесты.

Основные сигнатуры:

```rust
// core: конкретные поля берутся из одноимённых JS-объектов, IDs = ExternalId.
struct ProductionData {
    catalog: ProductionCatalog,
    resource_operations: Vec<ResourceOperation>,
    factories: Vec<FactoryEnterprise>, artisans: Vec<ArtisanEnterprise>,
    contracts: Vec<LaborContract>, expansions: Vec<RgoExpansion>,
    construction_assignments: Vec<LaborAssignment>,
    completed_construction: Vec<CompletedConstruction>,
    storage_facilities: Vec<StorageFacility>,
    storage_reservations: Vec<StorageReservation>,
    goods_reservations: Vec<GoodsReservation>,
    money_reservations: Vec<MoneyReservation>, obligations: Vec<Obligation>,
    committed_operations: BTreeMap<ExternalId, CommittedOperation>,
    phase_receipts: BTreeMap<PhaseKey, ProductionPhaseReceipt>,
}
struct ProductionRequest { expected_revision: u64, period: String,
    policies: ProductionPolicies, profile: ProductionProfile,
    maintenance_receipts: Vec<MaintenanceReceipt> }
// WorldState.production_data: ProductionData; authoritative current data.
// WorldState.enterprises stays a derived view refreshed when a phase changes an enterprise.
fn project_production(source: &SourceSnapshot, codec: &CodecContext)
    -> anyhow::Result<ProductionData>; // io
fn read_committed_operations(source: &SourceSnapshot, event_ids: &[ExternalId])
    -> anyhow::Result<Vec<CommittedOperation>>; // io, read-only and revision-pinned
fn execute_production_cycle(world: &mut WorldState, request: &ProductionRequest)
    -> Result<ProductionPhaseReceipt, ProductionError>; // sim
```

Задача 1 определяет перечисленные предметные типы с реальными используемыми полями из указанных JS-файлов, закреплёнными тестами декодирования. BTreeMap индексы упорядочиваются стабильным type-aware сравнением ExternalId, без объединения числа и строки. Public `ProductionRequest` не содержит произвольного количества выпуска/труда.

## Task 1: Производственные данные и настоящий input

**Files:** создать `core/src/{production,storage,operations}.rs`, `io/src/production_projection.rs`, `io/tests/production_projection_test.rs`; изменить `core/src/{lib,state,pop,enterprise}.rs`, `io/src/{lib,projection}.rs`; маленькие fixtures в `scripts/tests/fixtures/native-production/` и общий Rust helper `sim/tests/common/production.rs`.

**Interfaces:** `project_production(&SourceSnapshot,&CodecContext)->Result<ProductionData>`; дополнить POP `state_id`, stock `inventory_origin`/`method_kind` и поля нужных предметных объектов. `WorldState::refresh_enterprise_views()->Result<(),String>` строит старую компактную проекцию из текущих factories/artisans и финансовых индексов, не второй независимо изменяемый список владельцев/работников.

- [x] Сохранить исходники/manifests, EXE, launcher и fingerprints в before; runtime hash и revision/date сравнить с этапом 1. Если данные действительно поменялись параллельно, зафиксировать новую исходную границу вместо подмены старых результатов.
- [x] Написать RED `production_projection_reads_authored_terms`: codec fixture содержит rgo:iron, factory:steel (1.1 iron + 0.4 coal), artisan:steel, workforce/assignments/contracts, tenure/clerks/equipment, warehouse units/lease, held goods/space, obligations, policies, completed effects. Проверить точные IDs/значения/статусы и типы numeric/string ID; отсутствие поля не становится 0/ready.
- [x] Запустить `cargo test -p kingmaker-io --test production_projection_test`; проверить первичную ошибку нужного контракта/отсутствующего API.
- [x] Реализовать конкретные структуры и проекцию через существующий decoder/точный parse Money. Исходный catalogue берётся из настоящего `registry/productionCatalog`, без повторной генерации или исправления данных под Rust. `read_committed_operations` открывает read-only транзакцию, проверяет ту же campaign/revision и читает только запрошенные события через существующий codec; неизвестное событие не даёт proof, новый revision источника — явная ошибка. IO передаёт полученные immutable proofs в ProductionData перед фазой; новые native receipts записывает только native session после валидированного применения. Это сохранённые операции, а не готовый импорт результата новой фазы из JS.
- [x] Написать/проверить `authored_terms_reload_without_runtime_alias`: смена метода/коэффициента/уровня/назначения в отдельной test SQLite меняет следующие расчётные данные; original stock goods и чужие поля не подменяются. `refresh_enterprise_views` согласован с authoritative records.
- [x] Получить GREEN `production_projection_test` + существующие `codec_test`, `identity_graph_test`, `review_regression_test`; записать результат.

## Task 2: Точная денежная математика

**Files:** `sim/src/money_math.rs`, `sim/tests/money_math_test.rs`; workspace Cargo.toml/Cargo.lock, sim/Cargo.toml и sim/lib.rs.

**Interfaces:** `decimal_ratio(text:&str)->Result<(BigInt,BigInt),MoneyMathError>`; `round_ratio(n:&BigInt,d:&BigInt)->Result<BigInt,...>`; `multiply_money(Money,multiplier:f64)->Result<Money,...>`; `rate_pay(&MoneyRate,fte:f64,multiplier:f64)->Result<Money,...>`; `allocate_budget(&[BudgetRequest],Money)->Result<Vec<BudgetAllocation>,...>`. MoneyRate содержит точные numerator/denominator, запрос — external id и amount.

- [x] RED `rounding_and_budget_remainders_match_js`: roundRatio(1,2)=1, (-1,2)=-1; ставки 0 и exponent notation; запросы по 5 на IDs a/b при бюджете 3 дают 2/1, сумма ровно 3. Reverse input order сохраняет распределение по IDs. Отрицательная сумма/бюджет, non-finite множитель, denominator 0, результат вне i64 — ошибки.
- [x] Запустить целевой тест и подтвердить RED; затем добавить num-bigint/num-traits, фиксировать lock и реализовать функции по JS. Перевод finite f64 в десятичный текст должен совпадать с Number.toString JS в используемых расчётах; tiny oracle проверяет и пограничную/exponent запись, а не предполагает совпадение Rust formatter.
- [x] Получить GREEN `cargo test -p kingmaker-sim --test money_math_test --locked`; проверка промежуточных значений больше i64 при представимом результате не отвергает законную выплату.

## Task 3: Атомарная сессия операций и receipts

**Files:** `sim/src/operation_session.rs`, `sim/tests/operation_session_test.rs`, `core/src/operations.rs`, `sim/src/{lib,invariants}.rs`, `core/src/state.rs`.

**Interfaces:** `OperationSession::new(base:&WorldState)->Result<OperationSession,...>`; `stage(&mut self,op:&EconomicOperation)->Result<ApplyOutcome,...>`; `finish(self)->Result<PreparedChanges,...>`; `apply_prepared(world:&mut WorldState,changes:PreparedChanges)->Result<(),...>`. Session хранит только staged изменённые/new строки и receipts, не clone всей базы. Payroll/rent/debt/production — конкретные variants; unsupported emission/market operations явно отвергаются. StockDelta содержит external stock ID, delta и явную create metadata.

- [x] RED `mixed_operation_is_atomic`: balanced дебет+кредит и stock changes; переполнение последнего credit/нехватка последнего input оставляют accounts/stocks/obligations/receipts без изменений. По валютам суммы отдельно; unknown обычный payee остаётся ошибкой.
- [x] RED `lawful_payroll_credit_creates_only_derived_account`: работнику без кошелька законный payroll даёт net в personal_savings, налог — treasury:<state> operating_capital. Net+tax=gross debit; положительный credit создаёт канонический account ID из `[owner,currency,purpose]`, 0 не рисует деньги/счёт. Невалидный POP/state/договор или созданный произвольный credit отвергается.
- [x] RED `replay_precedes_current_funds_check`: повтор той же операции идемпотентен, иной payload под event ID отвергается. Phase receipt ключ campaign/revision/period и request policies/profile/maintenance не позволяет повторно применять изменённый запрос. Прежде применённый producer payroll должен быть проверен по реальному stored/native receipt, а не булевому флагу вызывающего кода.
- [x] Реализовать операции, разрешённые создания, reservations/obligations и исходные extras. Новые internal IDs выделять без усечения/коллизий, согласованно для всех новых строк и индексов; external IDs остаются авторитетными. Отложенный PreparedChanges проверяет revision/preconditions затронутых строк перед одной публикацией.
- [x] Дать валидатор `check_production_operations(before,after,receipt)->InvariantReport`: объясняет точные изменения денег/stock amounts/reservations/tenure и отсутствие изменений людей/авторских прав; ещё не перенесённая полная проверка прав остаётся NotMeasured, а не all_required_passed. GREEN: operation_session_test и прежние ledger/invariants tests.

## Task 4: Конечный склад, аренда и освобождение

**Files:** `sim/src/storage.rs`, `sim/tests/storage_production_test.rs`; модели core/storage.rs из задачи 1.

**Interfaces:** `storage_availability(world:&WorldState,facility:&ExternalId,goods:&ExternalId)->Result<StorageAvailability,...>`; `plan_storage_reservations(session:&mut OperationSession,requests:&[StorageRequest],period:&str)->Result<StorageBatchResult,...>`; `release_storage_reservation(session:&mut OperationSession,id:&ExternalId,event_id:&str)->Result<ReleaseReceipt,...>`. Локальный индекс space обновляется принятыми запросами; accepted/partial/blocked и registry changes не теряются.

- [x] RED `space_is_shared_and_held_space_is_not_free`: два goods используют одну общую capacity; stocks+held reservations уменьшают её по unit/space_per_unit. Несовместимый unit, unknown capacity/access — unknown/blocked, не бесконечное место. Воспроизвести существующий малый остаток из `scripts/tests/fixtures/kingmaker-storage-rounding.json`.
- [x] RED `partial_production_refunds_only_unused_space`: аренда 100, принят резерв 10, использовано 4; refund_unused требует вернуть только 60. При cash владельца склада 25 перечислить 25, obligation 35; остаток released_quantity=6, consumed_quantity=4. Повтор возврата не платит второй раз; полностью использованный резерв consumed без refund. Округление сверить JS.
- [x] Реализовать действующие storageAvailability/planStorageReservations/releaseStorageReservation в native session, включая оплаты, сохранённые lease terms/fee_paid_minor и частичные исходы. Не менять capacity автора на 12 недель заново: применяются уже заданные текущие правила/вместимости.
- [x] GREEN `cargo test -p kingmaker-sim --test storage_production_test --locked`; failed batch оставляет исходный мир/арендные деньги прежними.

## Task 5: Активности ДП, заводов и кустарей

**Files:** `sim/src/production_activities.rs`, `sim/tests/production_activities_test.rs`; core/production.rs модели из задачи 1.

**Interfaces:** `plan_production_activities(world:&WorldState,request:&ProductionRequest)->Result<Vec<ProductionActivity>,ProductionError>`; `validate_production_assignments(pops:&[PopRecord],assignments:&[LaborAssignment])->Result<(),...>`; `update_factory_tenure(factory:&FactoryEnterprise,actual:&[ActualAssignment])->Result<FactoryEnterprise,...>`. Производительность/назначения вычисляются из данных, не из simplified EnterpriseRecord.employees.

- [x] RED `rgo_uses_only_authored_resource_and_capacity`: grain/iron остаются rgo; 40000×approved initial multiplier, утверждённые completed expansions по 2000 и технологии; неправильный resource/method/land/staff/capacity — конкретная ошибка. Нельзя добавить processed output на ДП или второй ДП в той же провинции.
- [x] RED `factory_preserves_equipment_clerks_tenure_infrastructure`: validate staffing and technologies; отдельно изменить оборудование, clerks, tenure_Bmax/Tsat и completed railway multiplier 1.08, сравнить кандидатов с factories.cjs. 100 grain/50 iron прежнего положительного native теста остаются проверяемыми реальными rgo fixtures, а не фабриками без входов.
- [x] RED `artisan_is_independent_and_assignment_is_unique`: кустарь работает без завода; retooling/technology/метод и собственное сырьё ограничивают кандидата. POP workforce=1, два назначения time_fraction=.75 дают ошибку; worker group time>1 тоже. Строительное назначение участвует в том же лимите. Неизвестный species coefficient даёт blocked output=0, не 1.
- [x] Реализовать функции по rgo/factories/artisans/labor/building-effects; сохранить различие potential_output и output для последующего feasible plan. Для фабрики до actual production не начислять стаж.
- [x] GREEN `cargo test -p kingmaker-sim --test production_activities_test --locked`; test-state до планирования остаётся прежним.

## Task 6: Общая возможность работы и настоящая payroll

**Files:** `sim/src/{production_feasibility,production_payroll}.rs`, `sim/tests/{production_feasibility_test,production_payroll_test}.rs`.

**Interfaces:** `plan_feasible_production(session:&mut OperationSession,request:&ProductionRequest,candidates:&[ProductionActivity])->Result<FeasibleProductionPlan,...>`; `plan_prior_production_obligations(session:&mut OperationSession,period:&str)->Result<Vec<EconomicOperation>,...>`; `pay_production_labor(session:&mut OperationSession,request:&ProductionRequest,plan:&FeasibleProductionPlan)->Result<Vec<PayrollReceipt>,...>`. FeasibleProductionPlan содержит activities, input reservations, storage reservation IDs, idle obligations, payroll requests, blocked and purchase priorities. Возвращённые payroll receipts уже stage-applied в сессии.

- [x] RED `shared_iron_and_cash_are_fair`: два завода goal=1 steel, общий iron=1.1 и coal=.8 дают output=.5 каждому. Контракты 10/worker с idle guarantee 2, cash=20, tax=.1 дают общий gross=12, tax=2 по текущему индивидуальному JS округлению; reverse producers не даёт преимущество. Железо удержанное под стройку/чужая еда не покрывают входы.
- [x] RED `no_inputs_preserves_idle_tax_debt`: без входов оба завода output=0, planned production FTE=0, idle requests в сумме 4. При cash=0 образуется obligation 4 с исходным tax_owner и назначением, не пропадает договор. Прежние обязательства имеют предусмотренное право на деньги раньше новых требований.
- [x] RED `zero_rate_and_partial_payroll_match_js`: authored wage 0 и applied gross=0 допускают текущий JS receipt/FTE; ненулевой недоплаченный gross пропорционально уменьшает FTE. Policy wage/tax 0/1, неполный payroll, отсутствующий/чужой/старый receipt и повтор employment ID проверяются отдельно. Не назначать одну зарплату двум employment.
- [x] Реализовать алгоритм activity-plans: stable candidate/key order, оборудование/profile productivity без изменения authored hours, общий ресурс/денежные гарантии, finite storage, реальные payments/taxes/debts через задачи 2–4. Служебные выплаты общего бюджета сюда не добавляются.
- [x] GREEN targeted feasibility/payroll tests; детальные outputs совпадают с JS oracle, включая purchase priorities, плановые/фактические FTE и obligations.

## Task 7: Фактический выпуск и единый цикл

**Files:** заменить `sim/src/production.rs`, изменить sim/lib.rs и старые production_test.rs; добавить `sim/tests/production_cycle_test.rs`, `scripts/tests/production-oracle.cjs`; адаптировать `reports/review-2026-10-12/probes/tests/contracts.rs`, cli/server callers при изменённом experimental interface. Полный simulate_week остаётся error guard.

**Interfaces:** `execute_production_cycle(&mut WorldState,&ProductionRequest)->Result<ProductionPhaseReceipt,ProductionError>` из File Map; внутренний `stage_actual_production(session:&mut OperationSession,request:&ProductionRequest,plan:&FeasibleProductionPlan,paid:&[PayrollReceipt])->Result<ProductionResult,...>` принимает только законные результаты предыдущих задач. ProductionPhaseReceipt хранит запрос/период, operations, planned/actual activities, produced_by_kind, blocked, validation and registry changes; дата/revision не изменяются.

- [x] Перенести три RED R4 на реальную steel fixture: без iron/coal выпуска нет; при output=1 списано ровно 1.1 iron/.4 coal; reported output=созданный/пополненный настоящий owned stock. Их ignore снимается только после реального исправления; прочие deferred R5/R6/viewer не отмечать прошедшими.
- [x] RED `paid_fraction_real_stock_and_tenure`: plan output=1, подтверждено 50% FTE — actual=.5, входы .55/.2, соответствующий spent storage и tenure. Чужой stock того же goods в той же провинции неизменён; новый output stock имеет верные owner/province/facility/currency/unit/origin.
- [x] RED `whole_phase_error_and_repeat`: ошибка последнего производителя/возврата не публикует уже подготовленные зарплаты и stocks других производителей. Совпадающий ProductionRequest повторно возвращает receipt до новой feasibility и не увеличивает tenure; иные policies под тем же ключом вызывают conflict. Неизвестный output currency не угадывает первый счёт.
- [x] Реализовать цикл: prior obligations → актуальные кандидаты → feasible/storage → committed payroll → actual input/output → release remainder/tenure → проверка операций → единая RAM-публикация. Maintenance proofs поступают только из законных applied операций предыдущей фазы, никогда не из фиктивной единицы обеспеченности. Никаких полных SQL-записей/экспортов на фазу.
- [x] JS oracle использует именно foundation-функции на одинаковых логических fixtures. Сравнить accounts/stocks/leases/reservations/debts/FTE/tenure/event IDs/blocked; исключать только технические индексы/диагностику. Проверить смену авторских параметров и назначений в следующем production period без ручных derived-counter правок.
- [x] GREEN cycle/differential tests + production_test с сохранёнными 100/50 assertions + старые pipeline_gate/server/CLI refusal tests. Не удалять missing native phase из полного недельного guard, пока не перенесены её необходимые upstream-фазы.

## Task 8: Проверка, независимое ревью и публикация

**Files:** README, bin EXE, report/results в `reports/rust-production-2026-10-12`, текущие launcher только если их поддержанный режим реально изменился; progress.md/backup в work/rust-production-2026-10-12.

- [x] Одно свежее независимое итоговое ревью по spec/plan: деньги и float rounding, частичные оплаты, типы ID, люди/чужие права, shared constraints, освобождение резерва, повтор/rollback/tenure. Проверить замечания по оригинальным функциям, устранить относящиеся к этапу дефекты, повторить affected checks.
- [x] `cargo fmt --all -- --check`, `cargo test --workspace --locked`, small JS oracle и updated review contracts. На полном мире только read-only load новых проекций/поддержки значений; производственный цикл не исполнять. Отсутствующие правила остаются явными текстовыми статусами.
- [x] `cargo build --release --workspace --locked`; release small cycle/projection tests и отказ полной недели. Опубликованные EXE сравнить по SHA256 с release; source fingerprint/выбор базы/revision/date/runtime зафиксировать. Предыдущие EXE сохранить.
- [x] Обновить отчёт: конкретные исправления R4, точные differential cases, независимое ревью, что проверено на full read-only world, что ещё не перенесено, spec/plan paths и machine-readable results. Не выдавать время изолированной production фазы за время полноценной недели.

## Самопроверка плана и исполнение

Все разделы спецификации сопоставлены задачам: данные/изменения автора 1, точные суммы 2, атомарность/создания/история 3, склад/аренда 4, реальные виды производств/труд/стаж 5, общая feasibility/payroll/tax/debt 6, actual production/replay/differential 7, ревью/публикация/честные ограничения 8. Пять Review Focus имеют прямые тесты. Полная неделя и активная база остаются защищены; авторские числа не подменяются.

Исполнение сохраняет ранее выбранный автором способ: **здесь, задачи выполняет Codex с помощью Gemini для ограниченных поисков/черновиков, одно независимое ревью в конце**. У задачи сложные общие интерфейсы; независимые product-edits нескольких агентов одновременно не нужны. Перед реализацией автор рассматривает этот конкретный план.
