'use strict';
// Read-only SQLite audit. JSON summaries, never a full export or a weekly run.
const { DatabaseSync } = require('node:sqlite');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../..');
const jsRoot = path.join(root, '../Проект карты Тейванкаля');
const runtime = JSON.parse(fs.readFileSync(path.join(jsRoot, 'data/kingmaker/runtime.json'), 'utf8'));
const db = new DatabaseSync(runtime.dbPath, { readOnly: true });
const campaignKey = JSON.stringify(['string', runtime.campaignId]);
const rows = db.prepare('SELECT payload FROM km_entity WHERE campaign=? AND table_name=? ORDER BY ordinal');
const dictionaryRow = db.prepare('SELECT value FROM km_dictionary WHERE n=?');
const dictionaryCache = new Map();
function resolve(ref) {
  if (!ref || typeof ref !== 'object' || ref.__kingmaker_codec_v2__ !== 'id') return ref;
  if (!dictionaryCache.has(ref.value)) {
    const entry = dictionaryRow.get(ref.value);
    if (!entry) throw Error(`Missing dictionary ${ref.value}`);
    // km_dictionary.value is the JSON value itself, not an ID-key pair.
    const external = JSON.parse(entry.value);
    dictionaryCache.set(ref.value, external);
  }
  return dictionaryCache.get(ref.value);
}
function classifyRustStock(ext) {
  const s = String(ext || '').toLowerCase();
  if (s.includes('flour')) return 'Flour';
  if (s.includes('meat')) return 'Meat';
  if (s.includes('fish')) return 'Fish';
  if (s.includes('iron')) return 'Iron';
  if (s.includes('timber') || s.includes('wood')) return 'Timber';
  if (s.includes('coal')) return 'Coal';
  if (s.includes('cloth') || s.includes('textile')) return 'Cloth';
  if (s.includes('steel')) return 'Steel';
  return 'Grain';
}
const report = { runtime, tables: {}, examples: {}, stocks_by_author_goods: {}, stocks_incorrectly_classified_as_grain: {}, stock_province_substitution_rows: 0, source_treasury_accounts_by_purpose: {}, source_people: 0 };
for (const table of ['registry/territory', 'registry/pops', 'accounts', 'stocks',
  'registry/artisanEnterprises', 'registry/factoryEnterprises']) {
  let count = 0;
  for (const { payload } of rows.iterate(campaignKey, table)) {
    const record = JSON.parse(payload), v = record.value;
    count++;
    if (count === 1) report.examples[table] = {
      row_id: record.id, external_id: record.external_id,
      decoded_province: resolve(v.province_id), decoded_owner: resolve(v.owner_id),
      decoded_state: resolve(v.owner_state_id), decoded_currency: resolve(v.currency_id),
      decoded_pop_type: resolve(v.pop_type_id), decoded_method: resolve(v.method_id),
      amount_minor: v.amount_minor, amount: v.amount, workforce: v.workforce,
      employees: v.employed, beneficiary: resolve(v.beneficiary_pop_id),
      purpose: v.purpose || v.inventory_purpose,
      typed_province_dictionary_ordinal: v.province_id?.value,
      typed_currency_dictionary_ordinal: v.currency_id?.value,
      typed_owner_dictionary_ordinal: v.owner_id?.value
    };
    if (table === 'registry/pops') report.source_people += Number(v.size || 0);
    if (table === 'accounts' && /treasury|budget|state/i.test(v.purpose || '')) {
      report.source_treasury_accounts_by_purpose[v.purpose] = (report.source_treasury_accounts_by_purpose[v.purpose] || 0) + 1;
    }
    if (table === 'stocks') {
      const goods = String(resolve(v.goods_id)), qty = Number(v.amount || 0);
      const realProvince = Number(resolve(v.province_id));
      let rustProvince = Number(v.province_id?.value || 0) & 0xffff;
      const ext = String(record.external_id || '');
      const holding = ext.match(/holding_region_(\d+)/);
      if (holding) rustProvince = Number(holding[1]);
      else if (ext.startsWith('opening-food-reserve:')) {
        const tuple = JSON.parse(ext.slice(21));
        if (typeof tuple[1] === 'string' && /^\d+$/.test(tuple[1])) rustProvince = Number(tuple[1]);
      }
      if (rustProvince !== realProvince) report.stock_province_substitution_rows++;
      const bucket = report.stocks_by_author_goods[goods] ||= { rows: 0, quantity: 0 };
      bucket.rows++; bucket.quantity += qty;
      if (classifyRustStock(record.external_id) === 'Grain' && goods !== 'grain') {
        const wrong = report.stocks_incorrectly_classified_as_grain[goods] ||= { rows: 0, quantity: 0, example: {
          external_id: record.external_id, province: resolve(v.province_id), goods, qty } };
        wrong.rows++; wrong.quantity += qty;
      }
    }
  }
  report.tables[table] = count;
}
const campaign = db.prepare('SELECT revision,payload FROM km_campaign WHERE id=?').get(campaignKey);
const header = JSON.parse(campaign.payload).value;
report.campaign = { revision: campaign.revision, date: header.date, engine_hash: header.engine_hash, rules_hash: header.rules_hash };
report.full_world_weeks_executed = 0;
db.close();
const rustFiles = [];
function collect(dir) {
  for (const item of fs.readdirSync(dir, { withFileTypes: true })) {
    const f = path.join(dir, item.name);
    if (item.isDirectory()) collect(f);
    else if (f.endsWith('.rs') || item.name === 'Cargo.toml') rustFiles.push(f);
  }
}
collect(path.join(root, 'crates'));
rustFiles.push(path.join(root, 'Cargo.toml'), path.join(root, 'Cargo.lock'));
const hash = crypto.createHash('sha256');
for (const f of rustFiles.sort()) hash.update(path.relative(root, f)).update('\0').update(fs.readFileSync(f)).update('\0');
report.rust_source_sha256 = hash.digest('hex');
fs.writeFileSync(path.join(__dirname, 'source-world.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({ tables: report.tables, campaign: report.campaign,
  rust_source_sha256: report.rust_source_sha256,
  incorrectly_grain_rows: Object.values(report.stocks_incorrectly_classified_as_grain).reduce((s, x) => s + x.rows, 0),
  incorrectly_grain_goods: report.stocks_incorrectly_classified_as_grain }, null, 2));
