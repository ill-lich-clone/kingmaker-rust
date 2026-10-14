# Native dated supply implementation plan

Spec: docs/superpowers/specs/2026-10-13-rust-supply-design.md. Self-approved under user authorization. Execute inline with Superpowers, TDD and ONE final fresh review; no full-world weeks before assembled pipeline/checkpoint. Start product edits after Construction Stage8 final publication.

## Global constraints

Preserve current prices, fractional quotes, recipes, calendar, travel/return dates, capacity, goods/title/cash and budget protection. No working JS DB/runtime writes, migrations, manual assignment or artificial liquidity. Reuse actual native market/storage/budget/production/transport forecast helpers; no toy engine bypass. Keep one private phase state and canonical receipts, no full-world clone per order. Record missing diagnostics honestly. One final correction pass; publish verified CLI/server, then continue complete week/checkpoint/10-week goal.

## Review Focus

1. Two orders/ingredients share stock, route, terminals, fleet and warehouse without double allocation.
2. Partial arrivals round accumulated seller payment exactly once; cargo/escrow remain conserved.
3. Food forecasting distinguishes dated arrivals, delayed cargo, beneficial POPs and unavailable reserved stock.
4. State purchases cannot spend protected service/debt money or buy another state's/illegal goods.
5. Native save/replay and cohort/project claims retain shipping title/provenance; arbitrary payloads cannot authorize shipping.

### Task1: Missing authored transport state and closed authority

- [x] Inspect actual source required fields/registries; losslessly project only missing offers/recovery/geography/terms; reuse MobilityData.shipments and TransportData.
- [x] RED field/ID/unknown/source-change projection and native serde; arbitrary public dispatch/arrival/recovery denied; exact private operation authority, money and goods/cargo evidence.
- [x] Implement, targeted GREEN; readonly source counts if changed projection needs actual proof. No full-week advance or source save.

### Task2: Real dated shipment arrivals

- [x] RED date not due/due, lease/owned warehouse, partial/full/delayed storage, existing-stock mismatch and lost reservation; exact accumulated seller escrow settlement, carrier return date and provenance.
- [x] Closed prepare/apply/execute arrivals with readset, receipts and empty boundary; stale inputs/late failure/replay/mandatory evidence and mini serde checks.
- [x] Implement source formulas, compare ordinary and partial mini output to actual JS; goods stock+cargo and escrow/right conservation GREEN.

### Task3: Paid dispatch and autonomous supply

- [x] RED genuine offers/routes/blocked trade, fractional invoices, shared route/terminal/fleet/warehouse, lease, held and protected inputs, affordability/partial order, incoming nonfood quantity, automatic land paths and authored limitations.
- [x] Closed dispatch and autonomous derived orders from actual unmet market/production needs; actual freight/escrow/goods/reservations, original destinations/IDs, no caller-granted inventory.
- [x] Actual mini dispatch→dated arrival, replay/late failure, source parity and all invariants GREEN.

### Task4: Interruption, approved recovery and physical return

- [x] RED lawful actor/terms, stale plans, extra freight, storage release/new reserve, date hold, partial return/refund and wrong event; no unauthorised auto decisions.
- [x] Implement source APIs/readsets/history/provenance; mini serde and conservation GREEN. Explicitly report mechanisms with no natural activation.

### Task5: Food lead-time forecasting and local reserves

- [x] RED horizon travel+7, daily rate/7, actual on-hand/held/pipeline, delayed dates, beneficial/shared service POP stores, multiple routes and no duplicate orders.
- [x] Integrate real paid local reserve market and dispatch feedback; no artificial initial stocks, immediate foreign food or consumption twice. Source parity and queue shortage diagnostics GREEN.

### Task6: State raw procurement and income integration

- [x] RED service two-week protection/obligations/funding0..100, actual raw DP/state territory/sale offer/warehouse and exact limited budget, no foreign/private or free stock; repeats/late failures.
- [x] Use genuine local purchase executor with source eligibility; closed procurement receipts and accumulated money/goods explanation. Include required arrivals/reserves/supply/procurement history in income while rejecting missing/tampered evidence.
- [x] Actual mini production→market→dispatch/arrival→seller recognition and producer input provenance, all affected budget/market/income regressions GREEN.

### Task7: Final verification, review and publication

- [x] Whole workspace excluding unchanged costly loader; actual relevant projection proof retained. One fresh final review against before manifest/spec/plan/ledger and five Review Focus inputs; verified Important/Critical fixes RED→GREEN in ONE pass, Minor ledger.
- [x] Release checks/build and verified CLI/server hashes/report/machine results. Source DB/runtime unchanged; full-world health still unmeasured.
- [ ] Continue assembled dated week, atomic history/checkpoint/new-process continuation, autonomous X-week runner with progress/timing, then10 actual full-world weeks and factual diagnosis. Do not mark goal complete at this stage.

Preflight: arrivals precede ordinary mobility/hiring; reserves/supply/procurement follow local market and precede income. Shipping state shares existing stock/storage claims and cohorts; executor owns actual transport money/goods, forecast remains readonly. Construction refunds/claims remain stable. Source foundation-turn.cjs43,91–93 verified; Gemini ordering inference about a post-market arrival phase rejected in favour of actual source. No product changes until prior stage finalgate.
