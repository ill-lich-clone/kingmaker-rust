use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    pub path: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kingmaker-rust-fixture-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE km_dictionary(n INTEGER PRIMARY KEY,value TEXT); CREATE TABLE km_shared(kind TEXT,n INTEGER,payload TEXT); CREATE TABLE km_campaign(id TEXT PRIMARY KEY,revision INTEGER,payload TEXT,static_payload TEXT); CREATE TABLE km_entity(campaign TEXT,table_name TEXT,id_key TEXT,external_id TEXT,ordinal INTEGER,sequence INTEGER,payload TEXT); CREATE TABLE km_table_meta(campaign TEXT,table_name TEXT,max_ordinal INTEGER,next_sequence INTEGER);").unwrap();
        let f = Self { path };
        for (n, value) in [
            (0, json!("fixture")),
            (176, json!("COIN")),
            (177, json!("OTHER")),
            (240, json!("pop:1")),
            (241, json!("holding_region_004")),
            (242, json!("state_001")),
            (250, json!("artisans")),
            (251, json!("culture:one")),
            (252, json!("religion:one")),
            (901, json!("[\"pop:1\",\"COIN\",\"personal_savings\"]")),
            (
                902,
                json!("[\"holding_region_004\",\"COIN\",\"operating_capital\"]"),
            ),
            (1001, json!("salt")),
            (1002, json!("clay")),
            (1003, json!("medicines")),
            (1100, json!("stock:holding_region_004:pretend_grain")),
            (140080, json!(1000)),
            (140081, json!("1000")),
            (141569, json!(69)),
            (141570, json!("69")),
        ] {
            conn.execute(
                "INSERT INTO km_dictionary VALUES (?1,?2)",
                params![n, value.to_string()],
            )
            .unwrap();
        }
        let header = envelope(
            "@campaign",
            json!({"campaign_id":id(0),"revision":123,
            "date":{"year":307,"month":4,"day":12},"authored_metadata":{"keep":true}}),
        );
        conn.execute(
            "INSERT INTO km_campaign VALUES (?1,123,?2,?3)",
            params![
                key(),
                header.to_string(),
                envelope("@static", json!({})).to_string()
            ],
        )
        .unwrap();
        f.add(
            "registry/territory",
            140080,
            json!(1000),
            json!({"province_id":id(140080),
            "name":"Province 1000","owner_state_id":id(242),"controller_state_id":id(242)}),
        );
        f.add("registry/territory",141569,json!(69),json!({"province_id":id(141569),
            "name":"Province 69","owner_state_id":id(242),"controller_state_id":id(242),"goods_id":id(1001)}));
        f.add(
            "registry/pops",
            240,
            json!("pop:1"),
            json!({"pop_id":id(240),"province_id":id(140080),
            "pop_type_id":id(250),"currency_id":id(176),"culture_id":id(251),"religion_id":id(252),
            "size":100,"workforce":60,"employed":30,"unemployed":30,"literacy":0.333333333333,
            "militancy":0.0,"consciousness":0.0}),
        );
        f.add("accounts",901,json!("[\"pop:1\",\"COIN\",\"personal_savings\"]"),json!({"account_id":id(901),
            "owner_id":id(240),"currency_id":id(176),"purpose":"personal_savings","amount_minor":"100"}));
        f.add("accounts",902,json!("[\"holding_region_004\",\"COIN\",\"operating_capital\"]"),json!({"account_id":id(902),
            "owner_id":id(241),"currency_id":id(176),"purpose":"operating_capital","amount_minor":"5000"}));
        f.add(
            "stocks",
            1100,
            json!("stock:holding_region_004:pretend_grain"),
            json!({"opening_stock_id":id(1100),
            "owner_id":id(241),"province_id":id(141570),"goods_id":id(1001),"amount":12.5,
            "unit":"tonnes","inventory_purpose":"commercial","authored_extra":{"keep":true}}),
        );
        f
    }
    pub fn add(&self, table: &str, ordinal_id: u64, ext: Value, value: Value) {
        let mut record = envelope(table, value);
        record["id"] = json!(ordinal_id);
        record["external_id"] = ext.clone();
        self.add_record(table, ordinal_id, ext, record);
    }
    pub fn add_record(&self, table: &str, ordinal_id: u64, ext: Value, record: Value) {
        let conn = Connection::open(&self.path).unwrap();
        let ordinal: i64 = conn
            .query_row(
                "SELECT count(*) FROM km_entity WHERE table_name=?",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO km_entity VALUES (?1,?2,?3,?4,?5,?5,?6)",
            params![
                key(),
                table,
                format!("id:{ordinal_id}"),
                ext.to_string(),
                ordinal,
                record.to_string()
            ],
        )
        .unwrap();
    }
    #[allow(dead_code)]
    pub fn sql(&self, sql: &str) {
        Connection::open(&self.path)
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub fn id(n: u64) -> Value {
    json!({"__kingmaker_codec_v2__":"id","value":n})
}
pub fn envelope(table: &str, value: Value) -> Value {
    json!({"storage_schema_version":2,"table":table,"value":value,"bigint_fields":[],"extensions":{}})
}
pub fn key() -> String {
    "[\"string\",\"fixture\"]".to_string()
}
