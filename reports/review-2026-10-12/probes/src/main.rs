// Read-only inspection of the loaded full world. Does not call simulate_week.
use kingmaker_io::SqliteLoader;
use std::collections::{HashSet, HashMap};
fn main() {
    let path = std::env::args().nth(1).expect("Pass SQLite path explicitly");
    let world = SqliteLoader::load_from_path(path, "world-306").expect("Read-only load");
    let provinces: HashSet<_> = world.provinces.iter().map(|p| p.id).collect();
    let accounts: HashSet<_> = world.accounts.iter().map(|a| a.id).collect();
    let mut stocks_by_goods: HashMap<String, (usize, f64)> = HashMap::new();
    for s in &world.stockpiles {
        let total = stocks_by_goods.entry(format!("{:?}", s.goods)).or_default();
        total.0 += 1;
        total.1 += s.quantity;
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "campaign": world.campaign,
        "provinces": world.provinces.len(), "pops": world.pops.len(),
        "people": world.pops.iter().map(|p| p.size as u64).sum::<u64>(),
        "accounts": world.accounts.len(), "enterprises": world.enterprises.len(),
        "stockpiles": world.stockpiles.len(),
        "pop_province_not_found": world.pops.iter().filter(|p| !provinces.contains(&p.province_id)).count(),
        "stock_province_not_found": world.stockpiles.iter().filter(|s| !provinces.contains(&s.province_id)).count(),
        "enterprise_province_not_found": world.enterprises.iter().filter(|e| !provinces.contains(&e.province_id)).count(),
        "pop_account_not_found": world.pops.iter().filter(|p| p.account_id.is_some_and(|a| !accounts.contains(&a))).count(),
        "enterprise_account_not_found": world.enterprises.iter().filter(|e| e.account_ids.iter().any(|a| !accounts.contains(a))).count(),
        "pop_range_empty": world.provinces.iter().filter(|p| p.pop_range == (0,0)).count(),
        "accounts_with_treasury_prefix_not_a_binding_check": world.accounts.iter().filter(|a| a.owner_id.as_str().is_some_and(|s|s.starts_with("treasury:"))).count(),
        "stocks_by_goods": stocks_by_goods,
        "sample_pop": world.pops.first(), "sample_enterprise": world.enterprises.first(),
        "sample_province": world.provinces.first(), "sample_account": world.accounts.first(),
        "total_cash_minor_all_currencies_not_a_conservation_check": world.accounts.iter().map(|a| a.balance.units() as i128).sum::<i128>().to_string(),
        "full_world_weeks_executed": 0
    })).unwrap());
}
