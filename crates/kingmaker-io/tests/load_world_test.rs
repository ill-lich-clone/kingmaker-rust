use kingmaker_io::SqliteLoader;
use std::path::Path;

#[test]
fn test_loads_2679_provinces_from_sqlite() {
    let db_path = "d:/ЛИЧный хлам/🎲 ДнД для поляны/Карты тейванкаля/Фулл карта/Проект карты Тейванкаля/data/kingmaker/current-world.sqlite";
    if !Path::new(db_path).exists() {
        eprintln!("Warning: current-world.sqlite not found at expected path, skipping test");
        return;
    }

    let world = SqliteLoader::load_from_path(db_path, "world-306")
        .expect("World should load successfully from SQLite");

    println!("\n=== LOADED WORLD STATS ===");
    println!("Provinces:   {}", world.provinces.len());
    println!("Pops:        {}", world.pops.len());
    println!("Accounts:    {}", world.accounts.len());
    println!("Enterprises: {}", world.enterprises.len());
    println!("Stockpiles:  {}", world.stockpiles.len());
    let food_reserves = world
        .stockpiles
        .iter()
        .filter(|s| s.purpose == "food_reserve")
        .count();
    println!("Food reserves: {}", food_reserves);

    assert_eq!(
        world.provinces.len(),
        2679,
        "Must load exactly 2679 provinces"
    );
    assert!(
        world.pops.len() >= 39396,
        "Must load full population, got {}",
        world.pops.len()
    );
    assert!(
        world.accounts.len() >= 5413,
        "Must load accounts, got {}",
        world.accounts.len()
    );
    assert!(
        world.enterprises.len() >= 15000,
        "Must load artisan/factory enterprises, got {}",
        world.enterprises.len()
    );
    assert!(
        world.stockpiles.len() >= 100000,
        "Must load full stocks, got {}",
        world.stockpiles.len()
    );
    assert_eq!(world.campaign.campaign_id, "world-306");
    assert_eq!(world.campaign.revision, 20);
    assert_eq!(
        (
            world.campaign.year,
            world.campaign.month,
            world.campaign.day
        ),
        (306, 1, 1)
    );
    assert!(world.validate_links().errors.is_empty());
    let source = world
        .source
        .as_ref()
        .expect("Complete source envelopes retained");
    assert_eq!(world.accounts.len(), source.records("accounts").map(Result::unwrap).count());
    assert_eq!(world.stockpiles.len(), source.records("stocks").map(Result::unwrap).count());
    assert_eq!(world.pops.len(), source.records("registry/pops").map(Result::unwrap).count());
    // Read monetary/POP primitives directly from original wire fields, independent
    // of the typed projection helper. Do not infer parity from matching counts.
    let accounts: std::collections::HashMap<_, _> =
        world.accounts.iter().map(|a| (a.id.0, a)).collect();
    for row in source.records("accounts") {
        let row = row.unwrap();
        let raw: serde_json::Value = serde_json::from_str(&row.payload).unwrap();
        let a = accounts[&(raw["id"].as_u64().unwrap() as u32)];
        let field = |key: &str| {
            raw["value"]
                .get(key)
                .or_else(|| raw["extensions"].get(key))
                .unwrap()
        };
        for (key, actual) in [
            ("account_id", a.external_id.json()),
            ("owner_id", a.owner_id.json()),
            ("currency_id", a.currency_id.json()),
        ] {
            assert_eq!(
                actual,
                *source.dictionary_value(field(key)["value"].as_u64().unwrap()).unwrap().unwrap(),
                "{key} changed for {}",
                row.id_key
            );
        }
        assert_eq!(
            a.purpose,
            field("purpose").as_str().unwrap(),
            "Purpose changed for {}",
            row.id_key
        );
        assert_eq!(
            a.balance.units(),
            field("amount_minor")
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap(),
            "Money changed for {}",
            row.id_key
        );
    }
    let pops: std::collections::HashMap<_, _> = world
        .pops
        .iter()
        .map(|p| (p.external_id.clone(), p))
        .collect();
    for row in source.records("registry/pops") {
        let row = row.unwrap();
        let raw: serde_json::Value = serde_json::from_str(&row.payload).unwrap();
        let field = |key: &str| {
            raw["value"]
                .get(key)
                .or_else(|| raw["extensions"].get(key))
                .unwrap()
        };
        let ext = kingmaker_core::ExternalId::from_json(
            (*source.dictionary_value(field("pop_id")["value"].as_u64().unwrap()).unwrap().unwrap()).clone(),
        )
        .unwrap();
        let p = pops[&ext];
        assert_eq!(
            u64::from(p.size),
            field("size").as_u64().unwrap(),
            "People changed for {}",
            row.id_key
        );
        assert_eq!(
            p.workforce,
            field("workforce").as_f64().unwrap(),
            "Workforce changed for {}",
            row.id_key
        );
        assert_eq!(
            p.employed,
            field("employed").as_f64().unwrap(),
            "Employment changed for {}",
            row.id_key
        );
        assert_eq!(
            p.unemployed,
            field("unemployed").as_f64().unwrap(),
            "Unemployment changed for {}",
            row.id_key
        );
    }
    let stocks: std::collections::HashMap<_, _> =
        world.stockpiles.iter().map(|s| (s.id, s)).collect();
    for row in source.records("stocks") {
        let row = row.unwrap();
        let raw: serde_json::Value = serde_json::from_str(&row.payload).unwrap();
        let id = raw["id"].as_u64().unwrap() as u32;
        let s = stocks[&id];
        let v = &raw["value"];
        let resolve = |field: &str| {
            let value = v
                .get(field)
                .or_else(|| raw["extensions"].get(field))
                .unwrap();
            (*source.dictionary_value(value["value"].as_u64().expect("Real source reference")).unwrap().unwrap()).clone()
        };
        assert_eq!(
            s.goods.external_id().json(),
            resolve("goods_id"),
            "Goods changed for {}",
            row.id_key
        );
        assert_eq!(
            s.owner_id.json(),
            resolve("owner_id"),
            "Owner changed for {}",
            row.id_key
        );
        let province = resolve("province_id");
        let expected = province
            .as_u64()
            .unwrap_or_else(|| province.as_str().unwrap().parse::<u64>().unwrap());
        assert_eq!(
            u64::from(s.province_id.0),
            expected,
            "Location changed for {}",
            row.id_key
        );
        assert_eq!(
            s.quantity,
            v["amount"].as_f64().unwrap(),
            "Quantity changed for {}",
            row.id_key
        );
    }
    println!(
        "Source records: {}",
        source.table_names().unwrap().iter().map(|table| source.records(table).map(Result::unwrap).count()).sum::<usize>()
    );
    println!("Source dictionary: {}", source.dictionary_count());
    println!(
        "People: {}",
        world.pops.iter().map(|p| u64::from(p.size)).sum::<u64>()
    );
    println!(
        "POP accounts present: {}; legitimately absent in source: {}",
        world.pops.iter().filter(|p| p.account_id.is_some()).count(),
        world.pops.iter().filter(|p| p.account_id.is_none()).count()
    );
}
