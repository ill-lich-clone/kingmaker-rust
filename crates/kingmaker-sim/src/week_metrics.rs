//! Compact measurements from authenticated operations. Forecasts never count as actual output.
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{
    operations::{EconomicOperation, OperationKind},
    week::NativeWeekReceipt,
    *,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Default)]
struct Place {
    province: Option<ProvinceId>,
    state: Option<ExternalId>,
    class: Option<ExternalId>,
}
impl Place {
    fn scopes(&self) -> Vec<Value> {
        let mut rows = vec![json!("world")];
        if let Some(p) = self.province {
            rows.push(json!({"province":p.0}));
        }
        if let Some(s) = &self.state {
            rows.push(json!({"state":s}));
        }
        if let Some(c) = &self.class {
            rows.push(json!({"pop_class":c}));
        }
        rows
    }
}
#[derive(Clone)]
struct StockInfo {
    good: ExternalId,
    unit: Option<Value>,
    place: Place,
}
pub struct WeekSummary {
    campaign: CampaignHeader,
    cash: BTreeMap<String, (ExternalId, i128)>,
    stocks: HashMap<ExternalId, StockInfo>,
    actors: HashMap<ExternalId, Place>,
    project_ids: HashSet<ExternalId>,
}
#[derive(Debug, Serialize)]
pub struct WeekMetrics {
    pub period: String,
    pub start_date: kingmaker_core::calendar::GameDate,
    pub end_date: kingmaker_core::calendar::GameDate,
    pub rows: Vec<Value>,
    pub problem_events: Vec<Value>,
    pub shipment_events: Vec<Value>,
    pub project_operations: Vec<EconomicOperation>,
    pub production_income: Vec<Value>,
    pub artisan_income: Vec<Value>,
    pub construction_summary: Vec<Value>,
    pub financial_obligations: Vec<Value>,
    pub budget_requirements: Vec<Value>,
    pub not_measured: Vec<String>,
}
enum Amount {
    Quantity(f64),
    Minor(i128),
    Count(u64),
}
#[derive(Default)]
struct Rows(BTreeMap<String, (Value, Amount)>);
impl Rows {
    fn add(&mut self, mut key: Value, amount: Amount) -> OpResult<()> {
        let id = serde_json::to_string(&key)?;
        if let Some((_, old)) = self.0.get_mut(&id) {
            match (old, amount) {
                (Amount::Quantity(a), Amount::Quantity(b)) => *a += b,
                (Amount::Minor(a), Amount::Minor(b)) => {
                    *a = a
                        .checked_add(b)
                        .ok_or_else(|| error("Metric money overflow"))?
                }
                (Amount::Count(a), Amount::Count(b)) => {
                    *a = a
                        .checked_add(b)
                        .ok_or_else(|| error("Metric count overflow"))?
                }
                _ => return Err(error("Metric dimensional mismatch")),
            }
        } else {
            key["status"] = json!("measured");
            self.0.insert(id, (key, amount));
        }
        Ok(())
    }
    fn quantity(
        &mut self,
        name: &str,
        place: &Place,
        good: &ExternalId,
        unit: &Option<Value>,
        kind: Option<&str>,
        q: f64,
    ) -> OpResult<()> {
        if !q.is_finite() || q < 0. {
            return Err(error("Nonfinite/negative measured quantity"));
        }
        for scope in place.scopes() {
            self.add(json!({"scope":scope,"name":name,"goods_id":good,"unit":unit,"production_kind":kind}),Amount::Quantity(q))?;
        }
        Ok(())
    }
    fn money(
        &mut self,
        name: &str,
        place: &Place,
        currency: &ExternalId,
        purpose: Option<&str>,
        n: i128,
    ) -> OpResult<()> {
        for scope in place.scopes() {
            self.add(
                json!({"scope":scope,"name":name,"currency_id":currency,"purpose":purpose}),
                Amount::Minor(n),
            )?;
        }
        Ok(())
    }
    fn scalar(&mut self, name: &str, place: &Place, n: f64) -> OpResult<()> {
        if !n.is_finite() || n < 0. {
            return Err(error("Invalid measured scalar"));
        }
        for scope in place.scopes() {
            self.add(json!({"scope":scope,"name":name}), Amount::Quantity(n))?;
        }
        Ok(())
    }
    fn count(&mut self, name: &str, place: &Place, n: u64) -> OpResult<()> {
        for scope in place.scopes() {
            self.add(json!({"scope":scope,"name":name}), Amount::Count(n))?;
        }
        Ok(())
    }
    fn finish(self) -> OpResult<Vec<Value>> {
        self.0
            .into_values()
            .map(|(mut row, n)| {
                row["amount"] = match n {
                    Amount::Quantity(q) => {
                        if !q.is_finite() {
                            return Err(error("Metric quantity overflow"));
                        }
                        json!(q)
                    }
                    Amount::Minor(n) => json!(n.to_string()),
                    Amount::Count(n) => json!(n),
                };
                Ok(row)
            })
            .collect()
    }
}
fn places(w: &dyn WorldRead) -> HashMap<ExternalId, Place> {
    let provinces = province_places(w);
    let mut rows = w.pops().values()
        .map(|p| {
            (
                p.external_id.clone(),
                Place {
                    province: Some(p.province_id),
                    state: p.state_id.clone(),
                    class: Some(p.pop_type_id.clone()),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    for p in w.provinces().values() {
        if let Some(state) = &p.owner_state_id {
            let id: ExternalId =
                format!("treasury:{}", crate::needs::js_string(&state.json())).into();
            let entry = rows.entry(id).or_insert(Place {
                province: None,
                state: Some(state.clone()),
                class: None,
            });
            if entry.state.as_ref() != Some(state) {
                entry.state = None;
            }
        }
    }
    for e in w.resource_operations().values()
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base))
        .chain(w.artisans().values().map(|e| &e.base))
    {
        if let Some(owner) = &e.owner_id {
            if rows.get(owner).is_none_or(|p| p.class.is_none()) {
                let state = provinces.get(&e.province_id).and_then(|p| p.state.clone());
                let entry = rows.entry(owner.clone()).or_insert(Place {
                    province: Some(e.province_id),
                    state: state.clone(),
                    class: None,
                });
                if entry.province != Some(e.province_id) {
                    entry.province = None;
                }
                if entry.state != state {
                    entry.state = None;
                }
            }
        }
    }
    rows
}
fn province_places(w: &dyn WorldRead) -> HashMap<ProvinceId, Place> {
    w.provinces().values()
        .map(|p| {
            (
                p.id,
                Place {
                    province: Some(p.id),
                    state: p.owner_state_id.clone(),
                    class: None,
                },
            )
        })
        .collect()
}
fn province_place(provinces: &HashMap<ProvinceId, Place>, p: ProvinceId) -> Place {
    Place {
        province: Some(p),
        state: provinces.get(&p).and_then(|r| r.state.clone()),
        class: None,
    }
}
fn cash_totals(w: &dyn WorldRead) -> OpResult<BTreeMap<String, (ExternalId, i128)>> {
    let mut result = BTreeMap::new();
    for a in w.accounts().values() {
        let id = serde_json::to_string(&a.currency_id)?;
        let row = result.entry(id).or_insert((a.currency_id.clone(), 0i128));
        row.1 = row
            .1
            .checked_add(i128::from(a.balance.0))
            .ok_or_else(|| error("Cash total overflow"))?;
    }
    Ok(result)
}
pub fn freeze_week_summary(w: &dyn WorldRead) -> OpResult<WeekSummary> {
    let provinces = province_places(w);
    Ok(WeekSummary {
        campaign: w.campaign().clone(),
        cash: cash_totals(w)?,
        actors: places(w),
        project_ids: w.construction_projects().values()
            .map(|p| p.id.clone())
            .collect(),
        stocks: w.stockpiles().values()
            .map(|s| {
                (
                    s.external_id.clone(),
                    StockInfo {
                        good: s.goods.external_id(),
                        unit: s.unit.clone(),
                        place: province_place(&provinces, s.province_id),
                    },
                )
            })
            .collect(),
    })
}
fn id(v: Option<&Value>) -> OpResult<Option<ExternalId>> {
    v.filter(|v| !v.is_null())
        .map(|v| ExternalId::from_json(v.clone()).map_err(|e| error(&e)))
        .transpose()
}
fn minor(v: Option<&Value>) -> OpResult<Option<Money>> {
    v.filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
        .transpose()
}
pub fn collect_week_metrics(
    before: &WeekSummary,
    w: &dyn WorldRead,
    p: &NativeWeekReceipt,
) -> OpResult<WeekMetrics> {
    if before.campaign.campaign_id != p.campaign_id
        || before.campaign.revision != p.request.expected_revision
        || kingmaker_core::calendar::GameDate::from(&before.campaign) != p.start_date
        || w.campaign().revision != p.end_revision
        || crate::week_history::validate_week_history(w, &p.request)?.as_ref() != Some(p)
    {
        return Err(error(
            "Metrics require authentic current week and its actual opening summary",
        ));
    }
    let mut actors = before.actors.clone();
    let provinces = province_places(w);
    let pops = w.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    actors.extend(places(w));
    let mut stocks = before.stocks.clone();
    for s in w.stockpiles().values() {
        stocks.insert(
            s.external_id.clone(),
            StockInfo {
                good: s.goods.external_id(),
                unit: s.unit.clone(),
                place: province_place(&provinces, s.province_id),
            },
        );
    }
    let subjects = w.resource_operations().values()
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base))
        .chain(w.artisans().values().map(|e| &e.base))
        .map(|e| (e.id.clone(), e))
        .collect::<HashMap<_, _>>();
    let contracts = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect::<HashMap<_, _>>();
    let mut rows = Rows::default();
    let mut problems = vec![];
    let mut not_measured = vec![];
    not_measured.push("Перевозки: агрегация по государству/провинции — нет измерения; реальные события с ID маршрутов и участников сохранены в shipment_events".into());
    let roster = crate::labor_roster::build_labor_roster(w)?;
    let mut pools = HashMap::<ExternalId, (Place, Option<f64>)>::new();
    for job in w.jobs().values() {
        let job_id = id(job.get("job_id").or_else(|| job.get("id")))?
            .ok_or_else(|| error("Declared job metric identity absent"))?;
        let pool = id(job.get("capacity_pool_id"))?.unwrap_or(job_id);
        let capacity = job
            .get("capacity_pool_total")
            .or_else(|| job.get("slots_total"))
            .and_then(Value::as_f64);
        let place = job
            .get("province_id")
            .and_then(|v| ExternalId::from_json(v.clone()).ok())
            .and_then(|id| ProvinceId::from_external(&id).ok())
            .map(|p| province_place(&provinces, p))
            .unwrap_or_default();
        if let Some((_, old)) = pools.get(&pool) {
            if *old != capacity {
                return Err(error("Metric shared job capacity differs"));
            }
        } else {
            pools.insert(pool, (place, capacity));
        }
    }
    if pools.is_empty() {
        rows.scalar("free_declared_job_fte", &Place::default(), 0.)?;
    }
    for (id, (place, capacity)) in pools {
        if let Some(capacity) = capacity.filter(|c| c.is_finite() && *c >= 0.) {
            let occupied = roster.by_pool.get(&id).map(|p| p.occupied).unwrap_or(0.);
            rows.scalar("declared_job_capacity_fte", &place, capacity)?;
            rows.scalar(
                "free_declared_job_fte",
                &place,
                (capacity - occupied).max(0.),
            )?;
        } else {
            not_measured.push(format!("job pool {:?}: нет измерения capacity", id));
        }
    }
    for pop in w.pops().values() {
        let place = actors.get(&pop.external_id).unwrap();
        rows.count("people", place, u64::from(pop.size))?;
        rows.scalar("workforce", place, pop.workforce)?;
        rows.scalar("employed", place, pop.employed)?;
        rows.scalar("unemployed", place, pop.unemployed)?;
        if pop.account_id.is_none() {
            rows.count(
                "people_without_personal_account",
                place,
                u64::from(pop.size),
            )?;
        }
    }
    for a in w.accounts().values() {
        rows.money(
            "cash",
            actors.get(&a.owner_id).unwrap_or(&Place::default()),
            &a.currency_id,
            Some(&a.purpose),
            i128::from(a.balance.0),
        )?;
    }
    if let Some(calibration) = &w.price_calibration() {
        let currencies = w.accounts().values()
            .map(|a| &a.currency_id)
            .collect::<HashSet<_>>();
        for currency in currencies {
            if let Some(prices) = calibration
                .prices_by_currency
                .get(&crate::needs::js_string(&currency.json()))
            {
                for (good, price) in prices {
                    if let Some(price) = price.as_f64().filter(|p| p.is_finite() && *p > 0.) {
                        rows.add(json!({"scope":"world","name":"authored_unit_price","goods_id":good,"currency_id":currency}),Amount::Quantity(price))?;
                    }
                }
            }
        }
    }
    let started = w.construction_projects().values()
        .filter(|r| !before.project_ids.contains(&r.id))
        .count();
    rows.count("construction_started", &Place::default(), started as u64)?;
    for project in w.construction_projects().values() {
        let place = project
            .extra
            .get("province_id")
            .and_then(|v| ExternalId::from_json(v.clone()).ok())
            .and_then(|id| ProvinceId::from_external(&id).ok())
            .map(|p| province_place(&provinces, p))
            .unwrap_or_default();
        for scope in place.scopes() {
            rows.add(json!({"scope":scope,"name":"construction_status","construction_status":project.status}),Amount::Count(1))?;
        }
    }
    let end_cash = cash_totals(w)?;
    for (key, (currency, end)) in &end_cash {
        rows.money(
            "cash_change",
            &Place::default(),
            currency,
            None,
            *end - before.cash.get(key).map(|r| r.1).unwrap_or(0),
        )?;
    }
    for (key, (currency, opening)) in &before.cash {
        if !end_cash.contains_key(key) {
            rows.money("cash_change", &Place::default(), currency, None, -opening)?;
        }
    }
    for s in w.stockpiles().values() {
        rows.quantity(
            "inventory",
            &province_place(&provinces, s.province_id),
            &s.goods.external_id(),
            &s.unit,
            None,
            s.quantity,
        )?;
    }
    let prod = w.production_phase_receipts().values()
        .find(|r| r.request.period == p.request.period)
        .ok_or_else(|| error("Production metrics receipt absent"))?;
    for output in &prod.outputs {
        let subject = subjects
            .get(&output.subject_id)
            .ok_or_else(|| error("Measured producer missing"))?;
        let unit = Some(crate::production_validation::goods_unit(
            w,
            &output.goods_id,
        )?);
        let place = province_place(&provinces, subject.province_id);
        rows.quantity(
            "production",
            &place,
            &output.goods_id,
            &unit,
            Some(&output.method_kind),
            output.quantity,
        )?;
        if let Some(reason) = &output.blocked_reason {
            problems.push(json!({"period":p.request.period,"subject_id":output.subject_id,"province_id":subject.province_id.0,"code":"production_blocked","reason":reason,"actual_output":output.quantity,"actual_fte":output.fte}));
        }
    }
    let wanted = p
        .operation_evidence
        .iter()
        .map(|e| (&e.event_id, e))
        .collect::<HashMap<_, _>>();
    let mut project_operations = vec![];
    let mut shipment_events = vec![];
    for diagnostic in &p.diagnostics {
        if diagnostic.get("reason").is_some() || diagnostic.get("code").is_some() {
            let mut row = diagnostic.clone();
            row["period"] = json!(p.request.period);
            row["source"] = json!("native_week_receipt");
            problems.push(row);
        }
    }
    for record in w.committed_operations().values()
        .filter(|o| wanted.contains_key(&o.event_id))
    {
        let op: EconomicOperation = serde_json::from_value(record.value.clone())?;
        let recognized = op.meta.get("income_recognized") == Some(&json!(true))
            || matches!(
                op.kind,
                OperationKind::Payroll
                    | OperationKind::EnterpriseIncome
                    | OperationKind::EnterpriseDividend
                    | OperationKind::AdventurerIncome
            );
        for debit in &op.debits {
            let place = actors.get(&debit.owner_id).cloned().unwrap_or_default();
            rows.money(
                "cash_debits",
                &place,
                &debit.currency_id,
                Some(&debit.purpose),
                i128::from(debit.amount_minor.0),
            )?;
            if op.kind == OperationKind::Purchase && op.meta.get("consumed") == Some(&json!(true)) {
                let consumer = id(op.meta.get("pop_id"))?
                    .and_then(|id| actors.get(&id).cloned())
                    .unwrap_or_else(|| place.clone());
                rows.money(
                    "consumer_spending",
                    &consumer,
                    &debit.currency_id,
                    None,
                    i128::from(debit.amount_minor.0),
                )?;
            }
        }
        if matches!(op.kind, OperationKind::Payroll | OperationKind::DebtPayment) {
            let contract = id(op.meta.get("contract_id"))?
                .as_ref()
                .and_then(|id| contracts.get(id))
                .copied();
            let state = id(op.meta.get("budget_state_id"))?.or(contract
                .map(crate::budget_funding::public_state)
                .transpose()?
                .flatten());
            if let Some(state) = state {
                let sector = op
                    .meta
                    .get("budget_sector")
                    .or_else(|| contract.and_then(|c| c.extra.get("budget_sector")))
                    .and_then(Value::as_str)
                    .unwrap_or("нет измерения");
                for debit in &op.debits {
                    let place = Place {
                        state: Some(state.clone()),
                        ..Default::default()
                    };
                    rows.money(
                        "public_service_payroll_paid",
                        &place,
                        &debit.currency_id,
                        None,
                        i128::from(debit.amount_minor.0),
                    )?;
                    rows.money(
                        "public_service_payroll_paid_by_sector",
                        &place,
                        &debit.currency_id,
                        Some(sector),
                        i128::from(debit.amount_minor.0),
                    )?;
                }
            }
        }
        for credit in &op.credits {
            let place = actors.get(&credit.owner_id).cloned().unwrap_or_default();
            rows.money(
                "cash_credits",
                &place,
                &credit.currency_id,
                Some(&credit.purpose),
                i128::from(credit.amount_minor.0),
            )?;
            if recognized && credit.purpose == "personal_savings" {
                rows.money(
                    "recognized_net_income",
                    &place,
                    &credit.currency_id,
                    None,
                    i128::from(credit.amount_minor.0),
                )?;
            }
        }
        let taxes = if let Some(incomes) = op.meta.get("income_by_pop").and_then(Value::as_array) {
            incomes
                .iter()
                .map(|r| {
                    Ok((
                        id(r.get("pop_id"))?,
                        minor(r.get("tax_minor"))?.unwrap_or(Money::ZERO),
                    ))
                })
                .collect::<OpResult<Vec<_>>>()?
        } else {
            vec![(
                id(op.meta.get("pop_id"))?,
                minor(op.meta.get("tax_minor"))?.unwrap_or(Money::ZERO),
            )]
        };
        let mut credit_cursor = 0;
        for (person, tax) in taxes {
            if tax.0 > 0 {
                let currency = id(op.meta.get("currency_id"))?
                    .ok_or_else(|| error("Measured tax currency absent"))?;
                let place = person
                    .as_ref()
                    .and_then(|id| actors.get(id))
                    .cloned()
                    .unwrap_or_default();
                rows.money("taxes_paid", &place, &currency, None, i128::from(tax.0))?;
                // Source dividends put withholding immediately after the net credit.
                // A government's own property share is never counted as a tax.
                let net_credit = person.as_ref().and_then(|person| {
                    op.credits
                        .iter()
                        .enumerate()
                        .skip(credit_cursor)
                        .find(|(_, c)| &c.owner_id == person && c.currency_id == currency)
                        .map(|(i, _)| i)
                });
                let tax_credit = net_credit.and_then(|i| op.credits.get(i + 1)).filter(|c| {
                    c.currency_id == currency
                        && c.amount_minor == tax
                        && c.owner_id
                            .as_str()
                            .is_some_and(|id| id.starts_with("treasury:"))
                });
                if let Some(i) = net_credit {
                    credit_cursor = i + 1;
                }
                let state = tax_credit
                    .and_then(|c| actors.get(&c.owner_id))
                    .and_then(|p| p.state.clone());
                if state.is_none() {
                    not_measured.push(format!(
                        "state tax allocation: нет измерения; event {:?}, pop {:?}",
                        op.event_id, person
                    ));
                }
                rows.money(
                    "tax_receipts",
                    &Place {
                        state,
                        ..Default::default()
                    },
                    &currency,
                    None,
                    i128::from(tax.0),
                )?;
            }
        }
        if op.kind == OperationKind::Production {
            let subject = id(op.meta.get("subject_id"))?;
            let entity = subject.as_ref().and_then(|id| subjects.get(id));
            for delta in op.stock_changes.iter().filter(|d| d.delta < 0.) {
                let stock = stocks
                    .get(&delta.stock_id)
                    .ok_or_else(|| error("Measured production input stock absent"))?;
                let place = entity
                    .map(|e| province_place(&provinces, e.province_id))
                    .unwrap_or_else(|| stock.place.clone());
                let kind = op
                    .meta
                    .get("method_kind")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        entity.and_then(|e| {
                            w.production_catalog()
                                .as_ref()?
                                .methods
                                .iter()
                                .find(|m| m.method_id == e.method_id)
                                .map(|m| m.kind.as_str())
                        })
                    });
                rows.quantity(
                    "production_inputs",
                    &place,
                    &stock.good,
                    &stock.unit,
                    kind,
                    -delta.delta,
                )?;
            }
        }
        if op.kind == OperationKind::FoodConversion {
            for delta in &op.stock_changes {
                let stock = stocks
                    .get(&delta.stock_id)
                    .ok_or_else(|| error("Measured food conversion stock absent"))?;
                rows.quantity(
                    if delta.delta < 0. {
                        "food_conversion_inputs"
                    } else {
                        "food_conversion_output"
                    },
                    &stock.place,
                    &stock.good,
                    &stock.unit,
                    None,
                    delta.delta.abs(),
                )?;
            }
        }
        if op.kind == OperationKind::Purchase {
            let good = id(op.meta.get("goods_id"))?
                .ok_or_else(|| error("Measured purchase good absent"))?;
            let quantity = op
                .meta
                .get("quantity")
                .and_then(Value::as_f64)
                .ok_or_else(|| error("Measured purchase quantity absent"))?;
            let province = op
                .meta
                .get("province_id")
                .and_then(|v| ExternalId::from_json(v.clone()).ok())
                .and_then(|id| ProvinceId::from_external(&id).ok())
                .ok_or_else(|| error("Measured purchase province absent"))?;
            let unit = Some(crate::production_validation::goods_unit(w, &good)?);
            rows.quantity(
                if op.meta.get("consumed") == Some(&json!(true)) {
                    "consumer_quantity"
                } else {
                    "inventory_purchase_quantity"
                },
                &province_place(&provinces, province),
                &good,
                &unit,
                None,
                quantity,
            )?;
        }
        if matches!(
            op.kind,
            OperationKind::ShipmentDispatch
                | OperationKind::ShipmentArrival
                | OperationKind::ShipmentReturnArrival
        ) {
            let shipment = id(op.meta.get("shipment_id"))?;
            let shipment = shipment
                .as_ref()
                .and_then(|id| {
                    w.shipments().values()
                        .find(|s| s.get("id") == Some(&id.json()))
                })
                .ok_or_else(|| error("Measured shipment absent"))?;
            let good = id(shipment.get("goods_id"))?
                .ok_or_else(|| error("Measured shipment commodity absent"))?;
            let quantity = op
                .stock_changes
                .iter()
                .filter(|s| {
                    if op.kind == OperationKind::ShipmentDispatch {
                        s.delta < 0.
                    } else {
                        s.delta > 0.
                    }
                })
                .map(|s| s.delta.abs())
                .sum();
            rows.quantity(
                if op.kind == OperationKind::ShipmentDispatch {
                    "dispatch_quantity"
                } else if op.kind == OperationKind::ShipmentReturnArrival {
                    "return_arrival_quantity"
                } else {
                    "arrival_quantity"
                },
                &Place::default(),
                &good,
                &shipment.get("unit").cloned(),
                None,
                quantity,
            )?;
            let details: serde_json::Map<String, Value> = shipment
                .as_object()
                .ok_or_else(|| error("Shipment diagnostics require object"))?
                .iter()
                .filter(|(key, _)| {
                    key.as_str() == "id"
                        || key.ends_with("_id")
                        || key.ends_with("_date")
                        || matches!(
                            key.as_str(),
                            "unit" | "quantity" | "remaining_quantity" | "status" | "path"
                        )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            shipment_events.push(json!({"period":p.request.period,"event_id":op.event_id,
                "operation_kind":op.kind,"actual_quantity":quantity,"shipment":details}));
        }
        if matches!(
            op.kind,
            OperationKind::CapitalistInvestment
                | OperationKind::ConstructionMaterials
                | OperationKind::ConstructionContractFee
                | OperationKind::FactoryWorkingCapital
                | OperationKind::ProjectRefund
                | OperationKind::ConstructionCancelled
                | OperationKind::ConstructionProgress
        ) || (op.kind == OperationKind::Payroll && op.meta.get("project_id").is_some())
        {
            project_operations.push(op.clone());
        }
    }
    for shipment in w.shipments().values() {
        if let Some(q) = shipment
            .get("remaining_quantity")
            .and_then(Value::as_f64)
            .filter(|q| *q > 0.)
        {
            let good = id(shipment.get("goods_id"))?
                .ok_or_else(|| error("Cargo metric commodity absent"))?;
            let name = match shipment.get("status").and_then(Value::as_str) {
                Some("in_transit") if shipment.get("storage_delayed") != Some(&json!(true)) => {
                    "in_transit_quantity"
                }
                Some("in_transit") => "delayed_cargo_quantity",
                Some("returning") => "returning_cargo_quantity",
                _ => "other_held_cargo_quantity",
            };
            rows.quantity(
                name,
                &Place::default(),
                &good,
                &shipment.get("unit").cloned(),
                None,
                q,
            )?;
        }
    }
    let market = w.market_phase_receipts().values()
        .find(|r| {
            r.request.period == p.request.period
                && r.request.stage == kingmaker_core::market::MarketStage::ClosingMarket
        })
        .ok_or_else(|| error("Closing needs metrics absent"))?;
    let mut needs = BTreeMap::<String, (Value, f64, u64, u64, u64)>::new();
    for row in &market.consumption {
        let pop = id(row.get("pop_id"))?.ok_or_else(|| error("Needs metric POP absent"))?;
        let p = pops
            .get(&pop)
            .copied()
            .ok_or_else(|| error("Needs metric POP reference absent"))?;
        let place = actors.get(&pop).unwrap();
        let need = row
            .get("need_id")
            .cloned()
            .ok_or_else(|| error("Needs metric ID absent"))?;
        if row
            .get("fraction")
            .and_then(Value::as_f64)
            .is_some_and(|f| f < 1. - 1e-9)
            && problems.len() < 100
        {
            problems.push(json!({"pop_id":pop,"province_id":p.province_id.0,"need_id":need,"fraction":row["fraction"],"code":"unmet_need"}));
        }
        for scope in place.scopes() {
            let key = json!({"scope":scope,"need_id":need});
            let entry = needs.entry(key.to_string()).or_insert((key, 0., 0, 0, 0));
            if let Some(fraction) = row.get("fraction").and_then(Value::as_f64) {
                if !fraction.is_finite() || !(0. ..=1.).contains(&fraction) {
                    return Err(error("Invalid measured need fraction"));
                }
                entry.1 += fraction * f64::from(p.size);
                entry.2 += u64::from(p.size);
                if fraction < 1. - 1e-9 {
                    entry.4 += u64::from(p.size);
                }
            } else {
                entry.3 += u64::from(p.size);
            }
        }
    }
    let mut output = rows.finish()?;
    for (_, (mut row, total, measured, unmeasured, incomplete)) in needs {
        row["name"] = json!("need_fraction");
        row["amount"] = if measured > 0 {
            json!(total / measured as f64)
        } else {
            json!("нет измерения")
        };
        row["status"] = json!(if measured > 0 {
            "measured"
        } else {
            "not_measured"
        });
        row["measured_people"] = json!(measured);
        row["unmeasured_people"] = json!(unmeasured);
        row["incomplete_people"] = json!(incomplete);
        output.push(row);
    }
    if market.consumption.is_empty() {
        not_measured.push("needs: нет измерения (no frozen consumption rows)".into());
    }
    not_measured.extend(["poverty_threshold: не реализовано; need satisfaction and actual cash/income measured separately","full_property_graph: нет измерения","endogenous_price_reaction: нет измерения; authored calibration remains explicit","school_medical_outcomes: не реализовано","hunger_deaths: не реализовано","free_installed_output_capacity: нет отдельного измерения; actual output and employment retained"].map(str::to_owned));
    let production_income = w.producer_income().values()
        .filter(|r| r.get("period") == Some(&json!(p.request.period)))
        .cloned()
        .collect();
    let artisan_income=w.artisans().values().filter(|e|e.base.extra.get("income_history").is_some_and(|h|h.get("period")==Some(&json!(p.request.period)))).map(|e|json!({"id":e.base.id,"owner_id":e.base.owner_id,"province_id":e.base.province_id.0,"income_history":e.base.extra["income_history"],"loss_carry_minor":e.base.extra.get("loss_carry_minor"),"undistributed_profit_minor":e.base.extra.get("undistributed_profit_minor"),"working_capital":e.working_capital})).collect();
    let construction_summary = w.construction_projects().values()
        .map(|r| json!({"project_id":r.id,"status":r.status,"details":r.extra}))
        .collect();
    let financial_obligations=w.obligations().values().map(|r|json!({"id":r.obligation_id,"payer_id":r.payer_id,"payee_id":r.payee_id,"currency_id":r.currency_id,"purpose":r.purpose,"amount_minor":r.amount_minor})).collect();
    let budget_requirements = w.budget_phase_receipts().values()
        .find(|r| r.request.period == p.request.period)
        .map(|r| r.requirements.clone())
        .unwrap_or_default();
    Ok(WeekMetrics {
        period: p.request.period.clone(),
        start_date: p.start_date,
        end_date: p.end_date,
        rows: output,
        problem_events: problems,
        shipment_events,
        project_operations,
        production_income,
        artisan_income,
        construction_summary,
        financial_obligations,
        budget_requirements,
        not_measured,
    })
}
