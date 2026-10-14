# Next precision measurements (diagnosis only; not implemented)

The user clarified that50seconds was an illustration, not a specific observed operation. Current scenario is a complete native week on world-306/revision20 from the selected runtime.

Instrumentation should be coarse, static and nested, with unchanged code paths, no per-POP logs:
1. income_cycle::prepare (ArtisanOpening): capture_stock_provenance; plan_artisan_choices; apply choices; plan_artisan_working_capital; apply capital; make registry; invariant verification; prepare/apply read validation.
2. artisan_working_capital: build_opening_artisan_quotes; initial working_capital_requirements; enterprise loop; second requirements; capital allocation. Counters for number of enterprises, requirements inspected, quote candidates and method candidates. The64-step method search is conditional, not executed for every method.
3. investments::prepare_automatic_investments_guarded: assess_investment_opportunities; build indexes; investor/offer matching; contribute; activate_staff; registry; invariant verification. Aggregate offers/visits/allocations without serializing per-offer details.
4. mobility_cycle: four existing subphases, check_mobility_operations, account/stock reconciliation, projection adoption and Reads::capture/unchanged. For guards, label table-level call sites to map large serialized payloads to dependencies. Never disable validity checks.
5. warehouse_capacity: split storage.capacity.needs into build_need_orders, food conversion, and facility accumulation; then split need building only if dominant.
6. loading/checkpoint: source fetch/decode/projection and JSON/source export/write/flush, with allocation and physical IO counters if available. Existing source_payload_bytes measures logical bytes only.

Measured evidence currently identifies stage cost and guard wall time. Static quadratic loops, cloning and repeated parsing are hypotheses for counters, not established explanations for all elapsed time.

Gemini supplied read-only hypotheses from selected source excerpts. Reviewed: linear find inside the enterprise loop; cloned candidate quotes; bounded conditional search in choose_for_enterprise. No diagnostic source patches or optimization applied.
