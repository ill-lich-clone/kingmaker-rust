//! Borrowed population equality; only account bindings may change in these phases.
use kingmaker_core::{row_store::RowView, *};
#[derive(PartialEq)]
struct PeopleRef<'a> {
    extra: &'a kingmaker_core::production::Extra,
    state_id: &'a Option<ExternalId>,
    id: &'a PopId,
    external_id: &'a ExternalId,
    province_id: &'a ProvinceId,
    pop_type: &'a String,
    pop_type_id: &'a ExternalId,
    culture_id: &'a Option<ExternalId>,
    religion_id: &'a Option<ExternalId>,
    size: &'a u32,
    workforce: &'a f64,
    employed: &'a f64,
    unemployed: &'a f64,
    currency_id: &'a Option<ExternalId>,
    maintenance_payer_id: &'a Option<ExternalId>,
    culture: &'a String,
    religion: &'a String,
    literacy: &'a Option<f64>,
    militancy: &'a Option<f64>,
    consciousness: &'a Option<f64>,
}
impl<'a> From<&'a PopRecord> for PeopleRef<'a> {
    fn from(p:&'a PopRecord)->Self {
        let PopRecord { extra, state_id, id, external_id, province_id, pop_type, pop_type_id, culture_id, religion_id, size, workforce, employed, unemployed, currency_id, maintenance_payer_id, account_status:_, culture, religion, account_id:_, literacy, militancy, consciousness }=p;
        Self { extra, state_id, id, external_id, province_id, pop_type, pop_type_id, culture_id, religion_id, size, workforce, employed, unemployed, currency_id, maintenance_payer_id, culture, religion, literacy, militancy, consciousness }
    }
}
pub(crate) fn same_people_except_accounts(before:RowView<'_,ExternalId,PopRecord>,after:RowView<'_,ExternalId,PopRecord>)->bool {
    before.values().map(PeopleRef::from).eq(after.values().map(PeopleRef::from))
}
