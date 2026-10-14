# Verified static context (not runtime attribution)
Gemini 3.8 Flash High reviewed supplied mobility/read_guard source without tools. Exit0 and statusSUCCESS; output saved separately. Codex checked the following against originals:
- mobility_cycle.rs:88-91 executes migration arrivals, local labor, profession changes and migration departures sequentially. Current trace has only the outer phase.mobility boundary, so their individual elapsed times are unknown.
- read_guard.rs:8-31: guard.serialize includes Serialize traversal, JSON writer, SHA256 and cancellation probes. It is not a SHA256-only timer and cannot distinguish these costs.
- read_guard.rs:39-48 serializes each row of a RowRead selection; nested guards can occur. Self durations, not summed inclusive durations, attribute total cost.
- warehouse_capacity.rs:144-160 encloses build_need_orders + food conversion + consumer capacity aggregation. Its time is not exclusively need arithmetic.
- investments.rs:379-544 includes assessment/index preparation, investor/offer matching, contribution, staff activation, registry generation and invariant verification. Existing phase-level trace cannot attribute a long uninstrumented interval to one of these functions.
Rejected as an established finding: Gemini speculation about unstable iteration order causing hash mismatches; no observed mismatch or supporting runtime evidence.
No optimization or source instrumentation changes were applied.
