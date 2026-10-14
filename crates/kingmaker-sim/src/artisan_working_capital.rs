//! Actual opening finance. Forecasts never mint money; the closed income phase authorizes these transfers.
use kingmaker_core::world_read::WorldRead;
use crate::{
    artisan_choices::{
        build_opening_artisan_quotes, choice_stocks, choose_for_enterprise, stage_retooling,
        ArtisanChoicesPlan,
    },
    budget_funding::error,
    income_capital::working_capital_requirements,
    money_math::multiply_money,
    needs::js_string,
    operation_session::{OpResult, OperationSession},
    producer_income::effective_income_policies,
    profit_rights::integer,
};
use kingmaker_core::{
    income::IncomeRequest, market::MarketProfile, operations::EconomicOperation, ExternalId, Money,
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::json;
use std::collections::HashMap;

fn money(n: i128) -> OpResult<Money> {
    i64::try_from(n)
        .map(Money)
        .map_err(|_| error("Artisan capital money overflow"))
}
fn transfer(
    event: String,
    owner: &ExternalId,
    currency: &ExternalId,
    amount: Money,
) -> OpResult<EconomicOperation> {
    let funding = amount.0 > 0;
    let n = money(i128::from(amount.0).abs())?;
    serde_json::from_value(json!({"event_id":event,"type":if funding {"artisan_capital_funding"}else{"artisan_capital_release"},"income_recognized":false,"debits":[{"owner_id":owner,"currency_id":currency,"purpose":if funding {"personal_savings"}else{"operating_capital"},"amount_minor":n}],"credits":[{"owner_id":owner,"currency_id":currency,"purpose":if funding {"operating_capital"}else{"personal_savings"},"amount_minor":n}]})).map_err(Into::into)
}
pub(crate) fn plan_artisan_working_capital(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    profile: &MarketProfile,
) -> OpResult<ArtisanChoicesPlan> {
    let mut result = ArtisanChoicesPlan {
        operations: vec![],
        enterprises: w.artisans().iter().cloned().collect::<Vec<_>>(),
        diagnostics: vec![],
    };
    if result.enterprises.is_empty() {
        return Ok(result);
    }
    let quotes = build_opening_artisan_quotes(w, profile)?;
    let initial = working_capital_requirements(w, &quotes)?;
    let stocks = choice_stocks(w)?;
    let mut session = OperationSession::new(w)?;
    for e in &mut result.enterprises {
        let Some(req) = initial.entities.iter().find(|req| req.id == e.base.id) else {
            continue;
        };
        if req.possible
            || e.pending_retooling.is_some()
            || e.base.extra.get("automatic_choice") != Some(&json!(true))
            || quotes.is_empty()
        {
            continue;
        }
        let owner = &req.owner_id;
        let currency = &req.currency_id;
        let budget = money(
            i128::from(session.cash(owner, currency, "operating_capital").0)
                + i128::from(session.cash(owner, currency, "personal_savings").0),
        )?;
        let local = quotes
            .iter()
            .filter(|q| {
                q["province_id"] == json!(e.base.province_id.0)
                    && q["currency_id"] == currency.json()
            })
            .cloned()
            .collect::<Vec<_>>();
        let choice = choose_for_enterprise(w, e, &local, Some(budget), &session, &stocks)?;
        if choice.method_id == e.base.method_id || choice.retooling.is_none() {
            continue;
        }
        let terms = choice.retooling.as_ref().unwrap();
        let cost = integer(&terms["cost_minor"])?
            .to_i64()
            .ok_or_else(|| error("Retooling capital overflow"))?;
        let purpose = terms["purpose"]
            .as_str()
            .ok_or_else(|| error("Retooling capital purpose missing"))?;
        let have = session.cash(owner, currency, purpose).0;
        if cost > have && purpose == "operating_capital" {
            let op = transfer(
                format!(
                    "artisan-retool-capital:{}:{}",
                    r.period,
                    js_string(&e.base.id.json())
                ),
                owner,
                currency,
                money(i128::from(cost) - i128::from(have))?,
            )?;
            session.stage_income(&op)?;
            result.operations.push(op);
        }
        let event = format!(
            "artisan-opening-choice:{}:{}",
            r.period,
            js_string(&e.base.id.json())
        )
        .into();
        result
            .operations
            .push(stage_retooling(w, e, choice, &mut session, event, true)?);
    }
    let mut state=crate::world_transaction::WorldTransaction::new(w);
    state.apply_phase(session.finish()?)?;
    state.replace_artisans(result.enterprises.clone())?;
    let requirements = working_capital_requirements(&state, &quotes)?;
    let policies = effective_income_policies(w, r);
    let mut session = OperationSession::new(&state)?;
    let mut principals = HashMap::new();
    let mut profits = HashMap::new();
    for g in &requirements.groups {
        let business = session
            .cash(&g.owner_id, &g.currency_id, "operating_capital")
            .0;
        let personal = session
            .cash(&g.owner_id, &g.currency_id, "personal_savings")
            .0;
        let excess = (business - g.minimum_minor.0).max(0);
        let releasable = if g.active {
            excess.min(g.personal_funded_minor.0)
        } else {
            excess
        };
        let mut amount = if g.minimum_minor.0 > business {
            (g.minimum_minor.0 - business).min(personal)
        } else {
            -releasable
        };
        let mut retained = g.retained_profit_minor.0;
        if !g.active && amount < 0 && retained > 0 {
            let gross = retained.min(-amount);
            let pop = state.pops()
                .iter()
                .find(|p| p.external_id == g.owner_id)
                .ok_or_else(|| error("Inactive artisan property income rights unresolved"))?;
            let rate = pop
                .state_id
                .as_ref()
                .and_then(|id| policies.taxes.get(&js_string(&id.json())))
                .copied()
                .unwrap_or(0.);
            if !rate.is_finite() || !(0. ..=1.).contains(&rate) {
                return Err(error("Inactive artisan property income tax invalid"));
            }
            let tax = multiply_money(Money(gross), rate)?;
            let mut credits = vec![
                json!({"owner_id":g.owner_id,"currency_id":g.currency_id,"purpose":"personal_savings","amount_minor":Money(gross-tax.0)}),
            ];
            if tax.0 > 0 {
                let state_id = pop
                    .state_id
                    .as_ref()
                    .ok_or_else(|| error("Inactive artisan tax state missing"))?;
                credits.push(json!({"owner_id":format!("treasury:{}",js_string(&state_id.json())),"currency_id":g.currency_id,"purpose":"operating_capital","amount_minor":tax}));
            }
            let op = serde_json::from_value(
                json!({"event_id":format!("artisan-retained-property:{}",json!([r.period,g.owner_id,g.currency_id])),"type":"enterprise_income","income_kind":"property","income_recognized":true,"pop_id":pop.external_id,"province_id":pop.province_id.0,"currency_id":g.currency_id,"paid_gross_minor":Money(gross),"tax_minor":tax,"debits":[{"owner_id":g.owner_id,"currency_id":g.currency_id,"purpose":"operating_capital","amount_minor":Money(gross)}],"credits":credits}),
            )?;
            session.stage_income(&op)?;
            result.operations.push(op);
            amount += gross;
            retained -= gross;
        }
        let principal = money((i128::from(g.personal_funded_minor.0) + i128::from(amount)).max(0))?;
        principals.insert((g.owner_id.clone(), g.currency_id.clone()), principal.0);
        profits.insert((g.owner_id.clone(), g.currency_id.clone()), retained);
        if amount != 0 {
            let kind = if amount > 0 {
                "artisan_capital_funding"
            } else {
                "artisan_capital_release"
            };
            let op = transfer(
                format!(
                    "{kind}:{}",
                    json!([
                        r.period,
                        g.owner_id,
                        g.currency_id,
                        g.minimum_minor.0.to_string()
                    ])
                ),
                &g.owner_id,
                &g.currency_id,
                Money(amount),
            )?;
            session.stage_income(&op)?;
            result.operations.push(op);
        }
        let shortage = (g.minimum_minor.0
            - session
                .cash(&g.owner_id, &g.currency_id, "operating_capital")
                .0)
            .max(0);
        if shortage > 0 {
            result.diagnostics.push(json!({"owner_id":g.owner_id,"currency_id":g.currency_id,"reason":"artisan_working_capital_shortage","shortage_minor":Money(shortage)}));
        }
    }
    let mut principal_left = principals.clone();
    let mut profit_left = profits.clone();
    for e in &mut result.enterprises {
        let Some(req) = requirements.entities.iter().find(|r| r.id == e.base.id) else {
            continue;
        };
        let key = (req.owner_id.clone(), req.currency_id.clone());
        let g = requirements
            .groups
            .iter()
            .find(|g| g.owner_id == req.owner_id && g.currency_id == req.currency_id)
            .unwrap();
        let last = g.ids.last() == Some(&e.base.id);
        let principal = if last {
            principal_left[&key]
        } else if g.minimum_minor.0 > 0 {
            (BigInt::from(principals[&key]) * BigInt::from(req.minimum_minor.0)
                / BigInt::from(g.minimum_minor.0))
            .to_i64()
            .ok_or_else(|| error("Artisan principal allocation overflow"))?
        } else {
            0
        };
        let old_profit = integer(
            e.base
                .extra
                .get("undistributed_profit_minor")
                .unwrap_or(&json!(0)),
        )?
        .to_i64()
        .ok_or_else(|| error("Artisan retained profit overflow"))?;
        let retained = if last {
            profit_left[&key]
        } else {
            old_profit.min(profit_left[&key])
        };
        *principal_left.get_mut(&key).unwrap() -= principal;
        *profit_left.get_mut(&key).unwrap() -= retained;
        e.base.payment_purpose = Some("operating_capital".into());
        e.base.extra.insert(
            "undistributed_profit_minor".into(),
            json!(retained.to_string()),
        );
        e.working_capital = Some(
            json!({"currency_id":req.currency_id,"minimum_minor":req.minimum_minor.0.to_string(),"requested_minor":req.requested_minor.0.to_string(),"personal_funded_minor":principal.to_string(),"possible":req.possible,"feasible_output":req.feasible_output,"period":r.period}),
        );
    }
    Ok(result)
}
