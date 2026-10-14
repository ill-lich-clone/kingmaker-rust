# Rust Market Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking. Метод уже выбран автором: выполнять здесь, один независимый итоговый review.

**Goal:** Перенести реальные нормы, оплату, продажи, потребление и пищевые конверсии на Rust с сохранением действующих правил и прав.

**Architecture:** Три атомарных именованных шага Maintenance / EarlyFood / ClosingMarket используют общую OperationSession и существующие money/storage helpers. Чистый планировщик клиринга поддерживает расчёт виртуальных пищевых предложений; публикуются только реально использованные конверсии и проверенные покупки. Подтверждённый производственный план передаёт исходные основания закупок сырья.

**Tech Stack:** Текущие Rust workspace, serde/serde_json, rusqlite, num-bigint, ICU ru-UA; Node JS oracle. Новые dependencies по этому плану не нужны.

**Spec:** `docs/superpowers/specs/2026-10-13-rust-market-design.md`, одобрена 13 октября 2026.

## Global Constraints

- Не менять экономические нормы, цены, ставки, начальные данные, бюджетные расходы и утверждённые параметры. Не вводить runtime-подмены ресурсов, ручные назначения или новые деньги.
- Полная неделя остаётся заблокированной. Недель/производственных фаз рабочего мира не запускать; small fixtures и read-only загрузка допустимы.
- Зерно → мука: `coefficient=1.03`; скот → мясо: `coefficient=5`. Клиринг: до 32 итераций, исходные epsilons; потребности: исходная нормализация до 12 знаков. Money(i64) проверяется, промежуточная арифметика BigInt.
- Региональная торговля работает только по текущему флагу; межрегиональная перевозка не становится мгновенной. Null-регионы отдельны, тип region_id сохранён.
- Неизвестная потребность/метрика сохраняется как null/«нет измерения», а не ноль. Денежные настройки и все неизвестные авторские поля остаются в данных.
- История операций единая: `production_data.committed_operations`; новые market receipts не создают второй ledger. People/FTE не меняются этим этапом.
- Для нового API сначала написать regression test, затем минимальные type/signature stubs для компиляции и зафиксировать семантический RED. Compile error сам по себе не считается проверкой экономического требования; реальное поведение реализуется после этого RED.
- Каждый новый модуль регистрировать в соответствующем `src/lib.rs` в той же задаче, где создаётся файл. Не откладывать подключение модулей до task 9.
- У проекта нет Git. До первой product-правки создать проверяемый backup/manifest; вместо commit отмечать завершённый checkpoint в `work/rust-market-2026-10-13/progress.md`.
- Работать в текущем согласованном checkout. Параллельное изменение источников сверять, чужие изменения не перезаписывать. Публикация актуальных EXE выполняется после проверок.
- Перед Cargo добавить только в окружение текущей команды PATH: `C:/Users/ill_lich/.cargo/bin;C:/Users/ill_lich/mingw64/bin;C:/Users/ill_lich/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained`. Не менять глобальный toolchain.

## Review Focus

1. Изменение набора предложений, валютного счёта или нормы между prepare/apply: отклонить подготовленный шаг до первой записи; тест task 3/8.
2. Промежуточная виртуальная мука и недостаток raw stock: не материализовать непроданное предложение, не оставить частично оплаченную конверсию; тест task 7.
3. Все деньги ушли на еду, а непокрытая другая basic-потребность содержания осталась: production не получает fraction=1 по одному успешному событию; тест task 8.
4. Нулевой выпуск из-за сырья/склада при положительном потенциале: будущий input demand не превращается в ноль; тест task 6.
5. Несколько проходов еды, fallback и военное замещение: один нормативный объём не съедается повторно, replay не зависит от изменившегося остатка; тест task 4/7/8.

## Файлы и интерфейсы

Новые core `market.rs` содержит source-модели и публичные results/requests; `market_data` добавляется к WorldState. IO `market_projection.rs` декодирует этот раздел без отдельного loader. Sim: `needs.rs` — нормы/заказы; `market_policy.rs` — scope, доступ и маски; `market_clearing.rs` — чистый клиринг; `market_inputs.rs` — производственные/строительные заказы и ограниченный land-path; `food_conversion.rs` — пищевые проходы; `market_cycle.rs` — подготовка/публикация шага; `market_invariants.rs` — проверка операций. Старый sim `market.rs` становится безопасным фасадом/отказом detached API.

`MarketStage` = Maintenance / EarlyFood / ClosingMarket. `MarketRequest` = expected_revision, period, stage, profile, optional priorities; `MarketProfile` содержит optional food_norm_multiplier и сохранённые extra. Unset и пустой явно заданный список приоритетов не смешивать. `MarketPhaseReceipt` содержит request/effective settings, исходный и остаточный спрос, fulfilled/purchased/consumed, события, конверсии, изменения складских броней/обязательств, maintenance receipts, diagnostics и audit.

`MarketData` = optional NeedsData, PriceCalibration, FoundationParameters, territory, maintenance agreements, sale offers и construction/institution terms, используемые закупками. Source-модели сохраняют flatten Extra и codec null/undefined. `NeedsData` содержит profiles/profileRefs/resolver; `ResolvedNeed` сохраняет количественные/статусные поля resolver. Авторитетный `kingmaker_core::market::MarketOrder` повторяет поля заказа JS; оставшийся старый toy MarketOrder не используется нативным solver.

Все исполняемые функции возвращают `operation_session::OpResult<T>`; `MarketError` — alias OperationError. Не добавлять собственную общую систему ошибок/событий/транзакций.

### Task 1: Source projection и контракты данных

**Files:** Create `crates/kingmaker-core/src/market.rs`, `crates/kingmaker-io/src/market_projection.rs`, `crates/kingmaker-io/tests/market_projection_test.rs`, `crates/kingmaker-sim/tests/common/market.rs`; Modify `crates/kingmaker-core/src/state.rs`, `crates/kingmaker-core/src/lib.rs`, `crates/kingmaker-io/src/projection.rs`, `crates/kingmaker-io/src/lib.rs`.

**Interfaces:** `project_market(source:&SourceSnapshot, codec:&CodecContext)->anyhow::Result<MarketData>`; подключить в существующий `projection::project`. `WorldState.market_data: MarketData`. Fixture helpers: `market_world()->WorldState`, `need_fixture()->(NeedsData, NeedProfileRef)`, `order(id:&str,buyer:&str,good:&str,quantity:f64,priority:&str,consume:bool)->MarketOrder`.

- [x] Сделать backup текущих crates/src/tests, Cargo manifests/lock, bin EXE, launchers и readonly runtime reference; сохранить relative paths/sha256/bytes в `backup-manifest.json`, проверить копии. Зафиксировать JS source hashes/Node/ICU и runtime workers=1/storage=memory. Не копировать полную SQLite.
- [x] RED `market_projection_preserves_ids_norms_and_unknowns`: codec fixture сохраняет `7` и `"7"`, weekly-норму `.01`, профиль, purpose, unknown extra, null/undefined, `region_id=7` и `"7"`. Отсутствие needs даёт None, не фиктивные нормы. Run `cargo test -p kingmaker-io --test market_projection_test`; получить содержательное расхождение, а не принять compile error за RED.
- [x] Реализовать перечисленные модели/projection; определить common fixtures: два ПОПа a/b по 1000, реальные personal accounts, продавец, две провинции региона r и третья other, котировка X, конечный склад, достаточные unit-test запасы. Это synthetic fixtures, не изменение рабочего мира.
- [x] GREEN той же командой; проверить существующие codec/projection tests. Checkpoint: данные загружаются без авторских подмен.

### Task 2: Реальные нормы и потребительские заказы

**Files:** Modify `crates/kingmaker-sim/src/needs.rs`; Create `crates/kingmaker-sim/src/market_policy.rs` (сначала scope), `crates/kingmaker-sim/tests/needs_native_test.rs`, `scripts/tests/market-oracle.cjs` (JS oracle только small JSON stdin); Modify common fixture.

**Interfaces:** `resolve_need_quantities(needs:&NeedsData, reference:&NeedProfileRef)->OpResult<Vec<ResolvedNeed>>`; `build_need_orders(world:&WorldState, profile:&MarketProfile)->OpResult<NeedBuildResult>`. `NeedBuildResult {rows,orders,diagnostics}`. Orders используют текущего ПОПа/его провинцию и авторитетный profileRef. Здесь реализуются `MarketScope::from_world(&WorldState)->OpResult<MarketScope>`, `key(ProvinceId)`, `same(ProvinceId,ProvinceId)`; task 5 использует готовый scope.

- [x] RED `weekly_food_is_authored_not_point_one`, `weekly_norm_missing_is_error`, `unknown_need_is_null`, `luxury_literacy_upgrade_and_military_match_js`, `slave_uses_agreement_not_wallet`, `data_change_changes_orders`. Assertions:
  ```rust
  assert_eq!(rows[0].gross_quantity, Some(10.0)); // 1000 × .01
  assert_eq!(food_with_multiplier_two, 20.0);
  assert!(missing_weekly_norm.is_err());
  assert_eq!(unknown.quantity, None);
  assert_eq!(slave_order.purpose, "sponsored_maintenance_reserve");
  ```
- [x] Run `cargo test -p kingmaker-sim --test needs_native_test`; реализовать точные resolver-ветви (weekly и явно существующие source branches), BigInt здесь не заменяет quantity f64. В oracle вызвать настоящий needs-resolver/buildNeedOrders; не реализовывать эталон заново и не читать рабочую DB.
- [x] GREEN и differential rows/orders: wealth weights `.50/.75/1/1.25/1.50`, literacy, upgrades, military inputs, zero/unknown H, missing refs, no needs diagnostics. Старые detached calculate-функции пока отдельно, замена в task 9. Checkpoint.

### Task 3: Авторизация покупок и конверсий в общей сессии

**Files:** Modify `crates/kingmaker-core/src/operations.rs`, `crates/kingmaker-sim/src/operation_session.rs`, `crates/kingmaker-sim/src/market_policy.rs`; Create `crates/kingmaker-sim/tests/market_operations_test.rs` и private unit tests рядом с авторизаторами.

**Interfaces:** OperationKind дополняется Purchase/FoodConversion. Private `authorize_purchase(&mut self, op:&EconomicOperation, order:&MarketOrder, proof:&PurchaseEvidence)->OpResult<()>`, `authorize_food_conversion(&mut self, op:&EconomicOperation)->OpResult<()>`, `configure_market(&mut self, request:&MarketRequest)->OpResult<()>`. `PurchaseEvidence` содержит основания реально выбранных партий, quote, seller-purpose и допустимого offer; derived conversion-offer обязан ссылаться на проверенный source offer/raw-stock и текущую реально staged conversion, а не на произвольное разрешение продажи. Внутренний MarketReadSet сохраняет прочитанные prices/norms/offers/territory/плательщиков/producer terms и наборы stocks/accounts/masks; preconditions проверяются через existing finish/apply_prepared до публикации.

Здесь же определяется `MarketPolicy::from_session(&OperationSession)->OpResult<MarketPolicy>` для canSell/sellerPurpose/held mask и approved offer provenance, а также `protected_artisan_inputs(&WorldState)->OpResult<HashMap<ExternalId,f64>>`. Session-версия использует актуальные stock overlays, не устаревшие opening amounts. Это готовая policy dependency task 4, а не временный all-can-sell режим.

- [x] RED `unauthorized_purchase_and_conversion_reject`, `seller_credit_is_paired_and_purpose_correct`, `metadata_cannot_shadow_operation`, `mixed_goods_units_do_not_buy_fake_quantity`, `market_read_change_rejects_before_cash_write`: требовать error и полный равный до/после WorldState; изменение котировки, появление eligible stock/account, изменение mask также отклоняют старый подготовленный шаг.
- [x] Run `cargo test -p kingmaker-sim --test market_operations_test`. Реализовать private capability и проверки quote/owner/actual deltas/paid_minor/units/consumed/warehouse; legitimate seller account creation только из проверенной продажи. Конверсия проверяет текущий raw source, рецепт 1.03/5, доступный объём и того же владельца/провинцию.
- [x] GREEN + `cargo test -p kingmaker-sim --test operation_session_test`; poisoned phase, конфликт событий, полностью идентичный replay и частичное intervening apply сохраняют прежние контракты. Checkpoint.

### Task 4: Чистый оплаченный клиринг

**Files:** Create `crates/kingmaker-sim/src/market_clearing.rs`, `crates/kingmaker-sim/tests/market_clearing_native_test.rs`; Extend JS oracle.

**Interfaces:** Internal `MarketView::from_session(&OperationSession)->OpResult<MarketView>`: количественный overlay/funds, source party metadata, policy/scope; не clone SourceSnapshot/целого WorldState. `plan_local_orders(view:&MarketView,period:&str,orders:&[MarketOrder],priorities:&[String])->OpResult<MarketClearResult>`. Result содержит план операций, fulfilment и PurchaseEvidence из task 3. `apply_market_result(session:&mut OperationSession,result:&MarketClearResult)->OpResult<()>` авторизует/применяет только реальные результаты.

- [x] RED `two_buyers_share_five_each`, `common_wallet_does_not_pay_twice`, `own_one_plus_paid_nine`, `priority_override_is_respected`, `seller_rounding_and_new_currency_match_js`, `food_fallback_and_military_do_not_double_eat`.
  ```rust
  assert_eq!(fulfilled, vec![5.0, 5.0]); // supply 10, two orders 10
  assert_eq!(own_plus_foreign_quantity, 10.0);
  assert_eq!(own_plus_foreign_paid_minor, Money(900)); // own 1, quote 1, buy 9
  assert_eq!(sum_debits, sum_credits);
  ```
- [x] Run `cargo test -p kingmaker-sim --test market_clearing_native_test`. Реализовать common buyer budget, progressive proposals 32 iterations, source sorting/ties, собственные партии перед оплатой, `:phaseN`, alternatives и military fallback. Funds/currency eligibility обновляются после фактических credits текущего прохода, как JS.
- [x] GREEN + differential перестановок, quote absence/foreign currency/0 cash/rounding tails, several sellers и политики приоритетов. У pure solver нет прав публиковать виртуальные партии. Checkpoint.

### Task 5: Регион, продажные права и складская покупка

**Files:** Modify `crates/kingmaker-sim/src/market_policy.rs`, `crates/kingmaker-sim/src/market_clearing.rs`, `crates/kingmaker-sim/src/needs.rs`; Create `crates/kingmaker-sim/tests/market_policy_storage_test.rs`; Reuse `crates/kingmaker-sim/src/storage.rs`.

**Interfaces:** Использует готовые scope/policy из tasks 2–3. Добавляет `plan_foundation_market(session:&mut OperationSession,period:&str,orders:&[MarketOrder],priorities:&[String])->OpResult<MarketClearResult>`, оборачивающий готовый solver/storage в правильном порядке.

- [x] RED `typed_region_and_null_scope`, `personal_and_beneficiary_stock_is_not_public`, `held_artisan_and_project_input_protected`, `partial_destination_storage_with_paid_rent`, `changed_scope_and_sale_right_changes_result`.
  ```rust
  assert!(!scope.same(province_numeric_region, province_string_region));
  assert_eq!(stored_quantity_in_capacity_three, 3.0);
  assert_eq!(foreign_personal_consumption, 0.0);
  assert_eq!(seller_party_province_after_sale, seller_party_province_before);
  ```
- [x] Run `cargo test -p kingmaker-sim --test market_policy_storage_test`. В wrapper бронь reserve_storage делается до stored-order clearing; consume_storage — только actual; release_storage — остаток. Сохранить source purchased-ID/create/unit/currency/beneficiary и approval/owner checks. Защита входов кустарей использует текущий ready/pending method, assignments и working_capital, не выдуманное постоянное удержание.
- [x] GREEN + `cargo test -p kingmaker-sim --test storage_production_test` и region/rights differential. Не хранить локально купленную мгновенно съеденную еду и не создавать shipment. Checkpoint.

### Task 6: Производственный спрос и ограниченная topology dependency

**Files:** Modify `crates/kingmaker-core/src/production.rs`, `crates/kingmaker-sim/src/production.rs`; Create `crates/kingmaker-sim/src/market_inputs.rs`, `crates/kingmaker-sim/tests/market_input_orders_test.rs`; Extend oracle.

**Interfaces:** `ProcurementActivity` сохраняет activity_id/subject_id/method_id/potential_output/labor_output/output из настоящего плана; `ProductionPhaseReceipt.procurement_activities: Vec<ProcurementActivity>`. `build_foundation_input_orders(world:&WorldState,production:&ProductionPhaseReceipt)->OpResult<Vec<MarketOrder>>`; `find_land_path(data:&MarketData,from:ProvinceId,to:ProvinceId)->OpResult<Option<Vec<ProvinceId>>>` только для surplus. Receipt должен совпадать campaign/period; потенциальный выпуск вычисляется производственным planner, не caller.

- [x] RED `zero_actual_output_still_orders_recipe_inputs`, `pending_retooling_and_free_owned_inputs`, `construction_status_limits_and_escrow`, `land_path_is_real_not_transport`.
  ```rust
  assert_eq!(steel_potential_one_iron_order, 1.1);
  assert_eq!(steel_potential_one_coal_order, 0.4);
  assert!(cancelled_project_orders.is_empty());
  assert_eq!(different_landmass_path, None);
  ```
- [x] Run `cargo test -p kingmaker-sim --test market_input_orders_test`; добавить вычисленные поля в receipt, сохранив прежние результаты/поля. Перенести buildFoundationInputOrders и только нужный buildingLimitReason по исходнику. Path: actual land neighbors, floating=false, одинаковый nonempty landmass, исходный BFS порядок; нет морских/воображаемых связей.
- [x] GREEN + affected production cycle/differential tests. Ни стройка, ни retooling, ни отправка груза этим helper не исполняются. Checkpoint.

### Task 7: Реальные пищевые проходы

**Files:** Create `crates/kingmaker-sim/src/food_conversion.rs`, `crates/kingmaker-sim/tests/food_conversion_native_test.rs`; Extend oracle.

**Interfaces:** `food_orders(orders:&[MarketOrder])->OpResult<Vec<MarketOrder>>`; `remaining_food_orders(orders:&[MarketOrder],purchases:&[EconomicOperation])->OpResult<Vec<MarketOrder>>`; `plan_food_conversion(session:&mut OperationSession,period:&str,orders:&[MarketOrder],stage:FoodStage)->OpResult<FoodResult>`. FoodStage = Early/Late/Surplus; FoodResult = receipts/converted/blocked. Использует tasks 4–6.

- [x] RED `grain_and_livestock_exact_real_ration`, `ready_food_first_and_unused_virtual_is_absent`, `unpaid_hunger_does_not_block_industry`, `grain_flour_merge_and_remaining_quota`, `surplus_paid_raw_demand_and_net_space`, `partial_military_and_maintenance_nonfood`, `conversion_error_rolls_back`.
  ```rust
  assert_close(grain_consumed_for_one_flour, 1.03, 1e-9);
  assert_close(livestock_consumed_for_one_meat, 5.0, 1e-9);
  assert_eq!(unpaid_conversion_quantity, 0.0);
  assert_eq!(unconsumed_virtual_party_quantity, 0.0);
  ```
  `assert_close(actual:f64,expected:f64,eps:f64)` добавить в common fixture.
- [x] Run `cargo test -p kingmaker-sim --test food_conversion_native_test`. Ready pass реально применяется в session. Converted pass рассчитывается на private MarketView с virtual offers; после вычисления used quantities сначала staging реальных conversion, затем проверенных покупок. Не создавать весь offered output. Surplus применяет rawDemand/foodWanted/path-length/net-space/rent по исходнику и сохраняет inventory_origin/unit.
- [x] GREEN + JS differential mixed owners/currencies, approved POP source-offer и его законный derived conversion-offer, held/raw produced earlier, rollback и idempotency. Заказы/quotes/food flag не меняются ради покрытия голода. Checkpoint.

### Task 8: Атомарный цикл, полное содержание и audit

**Files:** Create `crates/kingmaker-sim/src/market_cycle.rs`, `crates/kingmaker-sim/src/market_invariants.rs`, `crates/kingmaker-sim/tests/market_cycle_test.rs`, `crates/kingmaker-sim/tests/market_invariants_test.rs`; Modify `crates/kingmaker-sim/src/production.rs`, `crates/kingmaker-sim/src/production_validation.rs`, `crates/kingmaker-sim/src/operation_session.rs`.

**Interfaces:** Private-field `PreparedMarketPhase {changes,receipt,read_set}`; `prepare_market_phase(world:&WorldState,request:&MarketRequest)->OpResult<PreparedMarketPhase>`; `apply_market_phase(world:&mut WorldState,prepared:PreparedMarketPhase)->OpResult<MarketPhaseReceipt>`; public `execute_market_phase(world:&mut WorldState,request:&MarketRequest)->OpResult<MarketPhaseReceipt>`; `check_market_operations(before:&WorldState,after:&WorldState,receipt:&MarketPhaseReceipt)->InvariantReport`. Prepublish internal audit использует session overlays, не clone полного мира.

- [x] RED `maintenance_unfilled_second_need_limits_work`, `forged_stale_receipt_rejects`, `closing_uses_only_same_period_production`, `replay_identical_conflict_partial`, `read_change_after_prepare_is_atomic`, `audit_catches_free_goods_money_rights`. Повтор возвращает прежний receipt; конфликт не меняет деньги/запасы/брони/историю; по food fulfilled=10/10 и clothing=0/1 maintenance fraction=0, не 1.
- [x] Run `cargo test -p kingmaker-sim --test market_cycle_test --test market_invariants_test`. Собирать frozen все known basic demands, actual consumption по need_id и уникальным committed event IDs. Проверять интегрированный production receipt по полному спросу, не только присланным успешным событиям. При отсутствии количественной нормы сохранять not-measured; не создавать её.
- [x] Реализовать порядок Maintenance/EarlyFood/ClosingMarket точно из spec, стадийный replay до пересчёта и полную prepublication проверку; unknown/absent source data диагностируется отдельно от настоящего нулевого результата. Объяснять goods-deltas purchase/consumption/conversion, cash по валютам/счетам, storage changes, people/FTE неизменны. Конкретные operational rights проверять; full legal titles — NotMeasured.
- [x] GREEN + differential small sequence Maintenance→EarlyFood→native production→ClosingMarket. Это отдельные тестовые фазы, не игровая неделя. Проверить положительный, частичный, экономически blocked и полностью аварийный исходы по Mermaid. Checkpoint.

### Task 9: Безопасные фасады, итоговая проверка и EXE

**Files:** Modify `crates/kingmaker-sim/src/market.rs`, `crates/kingmaker-sim/src/needs.rs`, `crates/kingmaker-sim/src/lib.rs`, `crates/kingmaker-sim/tests/supply_test.rs`, `crates/kingmaker-cli/src/main.rs`, `crates/kingmaker-server/src/routes.rs` (только readiness metadata); Create `crates/kingmaker-sim/tests/market_differential_test.rs`, `work/rust-market-2026-10-13/publication-check.cjs`; Update report/results/ledger.

**Interfaces:** Публикуется `market_cycle::execute_market_phase`. Существующие detached `clear_market_orders` и `calculate_pop_demands[_with_reserves]` возвращают OpResult error без изменений: «use authoritative world/market request». У known callers тест toy priorities заменяется реальным paid-priority regression, supply route regression остаётся. CLI/API полную неделю по-прежнему отвергают до расчёта/изменения даты.

- [x] RED `detached_toy_helpers_cannot_execute_economy` и regressions week refusal/read-only API: явный error, прежние stocks/money/date/revision. Native entry advertised only as isolated; не устанавливать native_week_ready=true. Обновить exports/known callers без резервного пути к выдуманным нормам.
- [x] GREEN targeted tests, затем `cargo fmt --all -- --check`, `cargo test --workspace`. JS oracle и native fixtures должны совпасть по операциям/остаткам/ID/деньгам; до/после каждого fix rerun affected checks. Успешные дорогие full-read checks не повторять без новых причин.
- [x] Один свежий независимый итоговый review по spec+plan+diff/backup/tests. Исправить существенные замечания одним проходом; rerun affected checks. Minor замечания явно записать; не затевать второй обзор и несогласованные переделки.
- [x] `cargo build --release --workspace`; `cargo test --release -p kingmaker-sim --test market_differential_test --test market_cycle_test`. Read-only active-world release `--inspect` один раз проверяет фактическую загрузку добавленных data projections; измерить как загрузку, не неделю. При ошибке source-data/projection исправить/воспроизвести до публикации, не подменять данные. SQL допускается только read-only для header/fingerprint; полный digest/export не запускать.
- [x] Сверить source/EXE/runtime/header fingerprints, скопировать проверенные CLI/server release EXE в `bin`, проверить SHA256 и оба существующих launcher target. В report/results записать точные counts/checks, stage/spec/plan, ограничения, отсутствие world weeks и annual-save check; не перенести старые результаты как новые. Открыть report пользователю. Checkpoint complete только при фактически пройденной проверке.

## Самопроверка и handoff

Покрытие spec: данные/нормы — 1–2; деньги/atomic guards — 3; клиринг — 4; регионы/права/склады — 5; спрос/путь — 6; пищевые правила — 7; полное содержание/replay/audit — 8; прежние API, регрессии и рабочие EXE — 9. Каждый Review Focus закреплён указанными тестами. Общий store/runtime источник и неготовая полная неделя сохранены. Mermaid из spec проверяется при task 8 и обновляется вместе с любым утверждённым уточнением порядка.

После review автора применяется уже выбранный executing-plans. План не разрешает молча изменить JS-баланс, базу, бюджет, перевозки или реализовать полный ход. Если выявится противоречие настоящих правил, сначала сохраняется небольшой воспроизводимый пример и объясняется требуемое уточнение.

