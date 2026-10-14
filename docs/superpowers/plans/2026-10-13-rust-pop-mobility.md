# Rust POP Mobility Implementation Plan

> REQUIRED: inline executing-plans, TDD, один свежий независимый обзор всего этапа после выполнения.

Самостоятельно утверждено 2026-10-13 по разрешению пользователя. Spec: docs/superpowers/specs/2026-10-13-rust-pop-mobility-design.md. Приступить после завершения/обзора/публикации местного труда. Source backup/hash manifest, текущий checkout без Git. Исходный world-306/read-only runtime не меняются; никаких пересборок стартовых профессий и фиктивного финансирования.

## Интерфейсы и порядок

Native `MobilityData` хранит отсутствующие авторские требования профессий и связанные имущественные/перевозочные строки; уже существующие LaborData.pop_mobility, ProductionData назначения/склады/долги и MarketData.needs/territory/offers остаются единственными своими реестрами. Новые доходы/строительство/перевозки последующих этапов читают те же строки, без альтернативного исходного snapshot-реестра.

Закрытые planners: split, profession, departure, arrival, end history. Общий prepared/readset/receipt с canonical money/goods/claim events. Начало: arrivals → existing local opening → profession → departures; конец: history только с фактическими native payroll. Public полная неделя пока guarded.

### Task 1: Lossless rights and profession projection

- [x] RED проекции reference.professions, capitalistEntities/projectInvestments/productionBeneficiaries/factoryBeneficiaries/shipments; unknown fields, typed IDs и null/undefined; изменения авторских правил видны без runtime-подмен.
- [x] Добавить core mobility.rs и IO mobility_projection; определить общие реестры для будущих income/construction/shipping без дублирования уже существующих массивов. Отсутствующая информация не разрешает удалить чужие права.
- [x] GREEN roundtrip + affected loader tests (валидный full source loader переиспользовать до изменения actual IO или исходного мира).

### Task 2: Closed conserved cohort split

- [x] RED parent/child size/workforce/literate and oldest unemployment, rounding, exact savings, separate currencies, free physical groups; partial/full stocks and held goods; raw own artisan retention, property/lease/shipment blockers, valid lease access and full reference remap.
- [x] Port scalar source split/asset rules, стабильный SHA256 child ID, typed references, private financial authorization and lawful new accounts/stocks/claims; debt split без признания дохода. Индексы по владельцу/POP/ссылкам один раз на batch, без полного мира на каждого ребёнка.
- [x] GREEN JS oracle fixtures + negative invalid/replay/partial history/readset cases; деньги/товары/люди объяснены только операциями.

### Task 3: Unemployment and unpaid history

- [x] RED ageing oldest free portion/newly free portion, employed/in-transit exclusions, zero/partial actual pay, no duplicate completed period; inactive/archived jobs omitted, claims survive release.
- [x] Implement end history from actual roster/payroll, not planned wage or stale counters. Preserve source remainders/changed_period/unknown mobility fields.
- [x] GREEN actual source history oracle and two-period releases interaction.

### Task 4: Funded profession changes

- [x] RED age1,10%+remainder, explicit funded_vacancy qualification/protected classes, child own salary, exact complete preview hiring, partial retry, no money or slots twice across children.
- [x] Add optional candidate/offer scope to current native local planner, preserving unfiltered behavior. Implement source profession scope with shared wage/raw/storage reservations and bulk split publication; only accepted whole child receives changes. No manual profession assignments.
- [x] GREEN source tiny profession comparisons; child earns and produces same period, original author wage contracts unchanged.

### Task 5: Walking departures and due arrivals

- [x] RED directed permitted borders, land contact/same landmass, internal2/external3 weeks,5%+remainder, chosen funded destination and in-transit vacancy subtraction;7days/edge/correct calendar/no fee/no work in transit; due arrival updates territory/ref/province/state/currency correctly.
- [x] Indexed BFS and type/province vacancy scope, reuse source funded hiring forecast and exact source tie sorting. Arrival/departure immutable prepared changes and canonical events; goods retain real physical location.
- [x] GREEN source route/departure/arrival oracle; impossible path/rights rejection leaves state unchanged.

### Task 6: Atomic native mobility cycle and persistence readiness

- [x] RED stale readset, wrong phase order, duplicate/conflicting/orphan events, rollback after last split failure; local→profession→migration→budget→production; split plus title/claim/leased storage roundtrip.
- [x] Closed phase APIs/readsets, exact operation authorization, refresh account/enterprise/POP ranges only after complete validation. Preserve required history fields; dates/revision unchanged in isolated helpers.
- [x] GREEN relevant suites and full source RAM opening probe, no full weeks before other required phases.

### Task 7: Stage publication and next stage

- [x] One fresh independent final review; one Important/Critical fix pass with RED regressions, record remaining risk/rulings. Verify release CLI/server hashes and publish actual EXE.
- [x] Save stage report/results/spec/plan/probe logs/limitations and hand off to the next self-approved income plan. Continuing goal still requires investment/construction and dated supply/reserves, then complete week/checkpoint/10-week automated diagnostic run; these are not stage completion claims. Never equate isolated test success with healthy economics.

Review focus: (1) assets/title/claims survive both partial and full split; (2) physical people cannot work and migrate simultaneously; (3) complete funded preview, shared budgets and reservations; (4) earliest causal clock and directed border; (5) exact snapshot/replay/phase history and no raw source fallback after native changes. Mermaid procedure in spec is authoritative and updated alongside intentional rule changes.
