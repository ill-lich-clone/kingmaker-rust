# Rust Recovery — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Исправить загрузку и учёт первого этапа утверждённой спецификации, исключив подмены данных и ложный успешный ход.

**Architecture:** Существующие пять crates сохраняются. IO декодирует настоящее хранилище и строит типизированные индексы; исходные envelopes неперенесённых данных удерживаются без утраты и повторного массового декодирования. Ledger работает с точными адресами счетов; полный недельный entry point отказывает до реализации обязательных фаз, сохраняя мир.

**Tech Stack:** Rust 2021, имеющиеся serde/serde_json/rusqlite/tokio/axum; существующий JS codec используется только как проверочный oracle, не как скрытый обработчик игровой недели.

**Spec:** `docs/superpowers/specs/2026-10-12-rust-recovery-design.md`, одобрена автором 12 октября 2026.

## Global Constraints

- Баланс, авторские параметры, ресурсы, рецепты, рабочая сила, правила собственности, бюджеты, перевозки и четыре последних экономических исправления сохраняются.
- Базу не подгонять под Rust. Динамическое поведение исправляется в движке.
- Выбор базы и кампании остаётся в существующем `../Проект карты Тейванкаля/data/kingmaker/runtime.json`.
- Полные недели мира и годовая серия на этом этапе не запускаются.
- Активная SQLite открывается только для чтения; никакой миграции, перезаписи или новой рабочей кампании.
- Неперенесённые правила явно обозначаются. Первым этапом не объявлять готовность производства, рынка, бюджета, миграции, строительства, полного viewer API или сохранения.
- Git отсутствует: сохранить до правок копию исходников/manifests в `work/rust-recovery-2026-10-12/before`, фиксировать проверки и результаты; не выдумывать коммиты.

## Review Focus

1. Разный тип внешних ID: число `7` и строка `"7"` не должны объединяться; только адреса провинций имеют явное правило номера карты. Тесты задачи 1/2.
2. У ПОПа может не быть личного счёта по правилам содержания; несколько валют владельца не разрешают угадывать «первый счёт». Тесты задачи 2/3.
3. Неизвестные поля, shared-ссылки, BigInt и extensions не теряются; ошибка не превращается в нулевое значение. Тесты задачи 1/2.
4. Переполнение адресата, повторный event ID с другой суммой и ошибка после успешной операции не должны оставлять частичные изменения. Тесты задачи 3.
5. Чужая кампания, отсутствие фаз или иной runtime не должны скрыто продвигать дату, менять выбранную базу либо оставлять launcher на старом EXE. Тесты задачи 5/6.

## Task 1: Идентичность, товар и codec-v2

**Files:** создать `crates/kingmaker-core/src/identity.rs`, `crates/kingmaker-io/src/codec.rs`; изменить `core/src/{ids,enums,state,province,pop,enterprise,lib}.rs`, `io/src/lib.rs`; тесты `io/tests/codec_test.rs`, `core/tests/primitives_test.rs`, tiny oracle `scripts/tests/codec-oracle.cjs`.

**Interfaces:**
- `ExternalId`: строковое либо числовое JSON-значение с сохранённым типом, Eq/Hash/serde. `AccountAddress { owner_id: ExternalId, currency_id: ExternalId, purpose: String }`.
- `ProvinceId(u32)` — настоящий номер карты; `ProvinceId::from_external(&ExternalId) -> Result<ProvinceId>`. Только в этом методе допустима канонизация десятичной строки номера.
- `Goods::from_external(ExternalId) -> Goods`, `Goods::external_id(&self) -> ExternalId`: известные варианты + отдельная идентичность каждого прочего товара, без fallback Grain. Товар перестаёт быть Copy, если требуется владение строкой; вызывающие места используют clone, сохраняя смысл операций.
- `CodecContext` со словарём и shared-данными; `decode_record(&self, envelope: &serde_json::Value) -> Result<serde_json::Value>` сохраняет типизированные codec-маркеры значений, которые нельзя представить обычным JSON без потерь, и обрабатывает реальные правила extensions/экранирования.
- `StoredRecord { table_name, id_key, ordinal, sequence, payload }`, `SourceSnapshot`: исходные envelopes и campaign/static payload в неизменяемом общем хранилище (`Arc`), без полного экспорта.

- [x] Сохранить исходники/manifests и fingerprint, проверить неизменность рабочей конфигурации. Зафиксировать уже полученный RED baseline из review, не гоняя недели.
- [x] Написать тесты: разные `7`/`"7"`; province `1000`/`"1000"` → 1000; недопустимое усечение отклоняется; goods salt/clay/medicines остаются различными; opaque ID не задаёт товар; unknown dictionary ID — ошибка; shared/BigInt/extensions/escaped key сохраняют исходную семантику; отсутствующее поле отличается от нуля.
- [x] `cargo test -p kingmaker-io --test codec_test`: получить RED, проверить, что падают утверждения нужных контрактов.
- [x] Реализовать указанные типы и decoder по `state-codec.cjs`, не изобретая новую кодировку. Неподдерживаемую структуру отвергать до изменения данных. Числа и деньги не преобразовывать через f32 или `unwrap_or(0)`.
- [x] Получить GREEN; tiny Node oracle вызывает `require(...state-codec.cjs).decodeRecord(...)` с настоящим словарём, сравнивает логические значения ключ за ключом. Сохранить результаты задачи.

## Task 2: Чтение настоящего мира и финансовый граф

**Files:** переписать `crates/kingmaker-io/src/sqlite_loader.rs`, создать `io/src/runtime_config.rs`; расширить данные в `core/src/{state,province,pop,enterprise}.rs`; тесты `io/tests/{load_world_test,identity_graph_test,runtime_config_test}.rs` и `io/tests/fixtures`.

**Interfaces:**
- `RuntimeConfig::load(path: &Path) -> Result<RuntimeConfig>` читает исходный JSON; сохраняет выбранные storage/workers/campaign/database/checkpoint настройки. `SqliteLoader::load_from_path(path, campaign_id) -> Result<WorldState>` остаётся явным read-only entry point.
- `AccountBalance`: собственный ID + `AccountAddress` + точная Money. `WorldState::account_by_address(&AccountAddress) -> Option<&AccountBalance>`; индексы не смешивают владельца, валюту и account ID.
- ПОП содержит внешнюю идентичность, валюту, workforce/employed/unemployed и проверенную связь с личным счётом там, где она действительно есть; легальное отсутствие записывается явно. Предприятие содержит owner/payment purpose и связи со всеми его действительными счетами, не произвольный первый ID при нескольких валютах.
- Запас содержит собственный ID, Goods, owner/province/facility/unit/purpose/beneficiary и количество. Дополнительные данные остаются в `SourceSnapshot`, таблицы и metadata не исчезают.
- `WorldState::validate_links() -> ValidationReport` различает битую связь, законное отсутствие и неперенесённую проверку.

- [x] Построить маленькую SQLite по настоящему codec-v2, с неупорядоченными dictionary IDs, numeric/string province refs, отдельными владельцами/валютами/назначениями, sponsored POP, складом, beneficiary и неизвестными полями; эта фикстура независима от активной базы.
- [x] Написать RED: дата **307-04-12 / revision 123** читается точно; nonexistent campaign — ошибка; salt province **69** с непрозрачным ID и holding_region_004 остаётся солью в 69; POP province **1000**, artisans, culture/religion разрешаются правильно; currency не становится счётом; два валютных счёта не объединяются; sponsored POP не получает вымышленный счёт; pop range содержит правильные группы; повреждённая ссылка даёт table/ID в ошибке.
- [x] Реализовать последовательное read-only чтение словаря/shared/envelopes и корректные индексы. Не применять эвристики по external_id, не присваивать нулевые fallback. Сохранить metadata/порядок/неперенесённые таблицы в общем raw-хранилище, typed projection строить только там, где она нужна этапу.
- [x] `cargo test -p kingmaker-io --test identity_graph_test --test runtime_config_test`: GREEN; сравнить tiny projection с независимым JS oracle. Данные чужой кампании не попадают в выбранную.
- [x] Усилить существующий full-world load test: чтение без недели, все авторские goods/province/ownership/account addresses/amounts сверены с источником, суммы по валютам и размеры ПОПов сохранены; raw envelopes сравнивать потоково. Не сохранять полный JSON-экспорт и не вычислять ненужный полный digest. Записать реальные показатели, включая законное отсутствие счетов.

## Task 3: Атомарная и повторяемая бухгалтерия

**Files:** `crates/kingmaker-sim/src/ledger.rs`, `core/src/money.rs` при необходимости checked API; тесты `sim/tests/ledger_test.rs` и `sim/tests/ledger_contract_test.rs`.

**Interfaces:**
- `LedgerSession::from_accounts(accounts: &[AccountBalance]) -> Result<LedgerSession>` строит ledger с полными адресами; неизвестный счёт не создаётся при переводе.
- `TransferRequest { event_id: String, from: AccountAddress, to: AccountAddress, amount: Money, reason: String }`.
- `LedgerSession::transfer_once(request: TransferRequest) -> Result<TransferOutcome, LedgerError>`, `TransferOutcome::{Applied, AlreadyApplied}`. Один event ID с другим содержимым — конфликт; тот же запрос — без повторных изменений.
- Существующие тесты обычного перевода обновляются только под точные адреса: суммы 10 000/5000, перевод 2500, результаты 7500/7500 сохраняются. Исчезновение прежней упрощённой сигнатуры не меняет игровых правил.

- [x] RED: обычный перевод, self 100→100, negative rejected, missing source/destination, zero не создаёт счёт, wrong currency/purpose rejected, insufficient cash, checked overflow, retry unchanged, conflicting event ID rejected. Во всех отказах балансы и история равны исходным.
- [x] Проверить все предикаты и новые суммы до записи. Не использовать `get_balance(...).unwrap_or(ZERO)` как доказательство существования. Сохранять владельцев, валюты и назначения. Историю добавлять только после успешной проверки/применения.
- [x] `cargo test -p kingmaker-sim --test ledger_test --test ledger_contract_test`: GREEN. Повторить relevant review repros с новыми точными адресами, сохранив их требования.

## Task 4: Правдивые проверки состояния

**Files:** `crates/kingmaker-sim/src/invariants.rs`, экспорт `sim/src/lib.rs`; тесты `sim/tests/invariants_test.rs`.

**Interfaces:**
- `CheckStatus::{Passed, Failed, NotMeasured { reason }}` и `InvariantReport`: money/goods/people/labor/rights отдельно, детализация валют и нарушенных ID. `all_required_passed()` не принимает NotMeasured за успех.
- `check_conservation(before, after) -> InvariantReport` проверяет неизменённое состояние без операций и выявляет необъяснённые изменения. `check_with_operations(before, after, operations) -> InvariantReport` принимает только реализованные валидируемые операции ledger; неизвестные источники/списания — NotMeasured/ошибка, а не арифметическое разрешение любого роста.

- [x] RED: size 100→0 при том же числе строк не проходит; +1000 необъяснённого зерна не проходит; минус денег в одной валюте и плюс в другой не взаимозачитываются; смена владельца без операции не проходит; double employment/отрицательный свободный запас/битая ссылка обозначаются; неподдерживаемый валидатор явно NotMeasured.
- [x] Реализовать проверки с индексами по реальным ID и metadata. Сохранение товаров проверять по количествам, правам и отдельным местам/удержаниям, не по сумме несопоставимых единиц. Допустимые изменения требуют соответствующего валидатора; ещё не перенесённую демографию, производство и эмиссию не заменять выдуманным validator.
- [x] `cargo test -p kingmaker-sim --test invariants_test`: GREEN; неизменённый маленький мир даёт положительные результаты только по реально проверенным категориям.

## Task 5: Безопасный entry point, настоящая конфигурация и ответы API

**Files:** `crates/kingmaker-sim/src/pipeline.rs`, `cli/src/main.rs`, `server/src/{main,routes,state}.rs`, необходимые Cargo manifests; тесты `sim/tests/pipeline_gate_test.rs`, `server/tests/server_api_test.rs`, `cli/tests/cli_test.rs`.

**Interfaces:**
- `simulate_week(world, recipes) -> Result<WeekSummary, WeekError>`: первая проверка полноты фаз; на первом этапе `WeekError::IncompleteEngine { missing_phases }` до любой записи/клонирования года. Отдельная `advance_week_date(date) -> Result<Date>` сохраняет утверждённый 30-дневный календарь для unit-тестов.
- CLI: `--runtime <path>` выбирает файл существующей конфигурации; `--inspect` только загружает/проверяет. По умолчанию запрос недели завершается явной ошибкой, не печатает «Week Execution Completed». Нет флага скрытого обхода блокировки.
- Сервер использует тот же RuntimeConfig. Health отражает `read_only`/неполную поддержку, а не «готов полный движок». Map/province выдаются по правильным связям и внешним идентичностям. Чужой campaign ID → HTTP 404; valid execute при missing phases → HTTP 503 с кодом и списком, без `executed=true`.

- [x] RED: valid execute сохраняет все данные, date/revision и историю; invalid campaign ничего не выполняет; календарь **306-12-27 → 307-01-04** работает отдельно; CLI inspect читает указанный runtime, отсутствующая база/битый runtime явно ошибочны; состояние API не использует hardcoded старую кампанию; truthfulness не зависит от наличия пустого списка рецептов.
- [x] Удалить ложный успешный путь полной недели; полный список отсутствующих фаз привязать к реальному JS-конвейеру, не только к отсутствию recipes. Текущие experimental phase helpers остаются явно непроверенными, не вызываются полным entry point и не считаются исправленной экономикой.
- [x] Обновить старые серверные тесты согласно одобренному изменению: прежний пустой мир больше не продвигается успешно. Сохранить positive health/map/province assertions и добавленные проверки отрицательных путей; не превращать отсутствие механик в пустой успешный ответ.
- [x] `cargo test -p kingmaker-sim --test pipeline_gate_test; cargo test -p kingmaker-server --test server_api_test; cargo test -p kingmaker-cli`: GREEN.

## Task 6: Независимая проверка, публикация EXE и отчёт

**Files:** `README.md`, существующие Rust launcher `.cmd` и `bin/kingmaker-{cli,server}.exe`, `reports/rust-recovery-2026-10-12/report.md`, компактные `results.json`/журналы. Исходный review-отчёт сохраняется как история первоначальных результатов; обновлённые repros не удаляют оставшиеся дефекты.

- [x] Одно независимое ревью итогового diff/изменённых источников: spec/plan, сохранение значений/прав, ошибки/частичные исходы, repeat/currency/overflow. Проверить сведения Gemini напрямую; устранить относящиеся к этапу замечания и повторить затронутые проверки.
- [x] `cargo test --workspace --locked`, affected recovery tests, small JS oracle и read-only full-world audit. Проверить свежие результаты. Все остающиеся R4/R5/R6/полный viewer/сохранение перечислить отдельно; не делать «все 18 закрыты» при закрытом первом этапе.
- [x] `cargo build --release --workspace --locked`. Проверить готовый CLI на tiny config/SQLite: inspect и отказ недели без изменений; активный мир только inspect. Замерять загрузку отдельно, полный ход не измерять.
- [x] Сохранить заменяемые EXE, скопировать проверенные release EXE в `bin`, сверить хеши. Launcher использует актуальные bin и существующий runtime, сообщает read-only режим и ясную ошибку запроса недели. Не запускать полный мир и не открывать браузер автоматически для тестирования.
- [x] README исправляет заявленное ускорение неполного расчёта, объясняет фактическую поддержку. Отчёт содержит spec/plan, source/binary fingerprints, что проверено, что осталось и точные показатели загрузки. Подтвердить, что дата/revision/runtime/база не менялись.

## Самопроверка плана

Загрузку/идентичности покрывают задачи 1–2, бухгалтерию 3, правдивые проверки 4, отказ и единый runtime 5, актуальный запуск и проверку 6. Все пять Review Focus получили отдельные тесты. Неперенесённые системы не маскируются новой формулой или произвольным разрешением. Нет требования годового/многонедельного прогона и нет записи в рабочие данные.

Рекомендуемое исполнение: **здесь, по задачам, с одним независимым ревью в конце**. Gemini помогает с небольшими поисками, тестовыми черновиками и независимой проверкой; Codex проверяет выводы и применяет изменения с последствиями. План одобрен автором; первый этап выполнен. Проверки, ограничения и публикация зафиксированы в reports/rust-recovery-2026-10-12/report.md.
