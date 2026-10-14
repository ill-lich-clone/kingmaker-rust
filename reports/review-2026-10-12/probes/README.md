# Contract reproductions: historical audit and current first-stage boundary

The original run and its 1 pass / 18 failures remain in the parent report and `contract-tests.txt`. Unchanged original test/main sources are backed up in `work/rust-recovery-2026-10-12/before/original-review-probes`.

The current 19 tests use explicit identity/account metadata and the approved error-returning week interface. Assertions for production inputs, actual stocks, food consumption, ownership, paid purchases and viewer reports remain. Nine deferred tests carry explicit `ignore` reasons so the stage-one suite can be run separately. **An ignored test is an unresolved contract, not a pass.** Run them with `-- --ignored`; their failures are recorded in `work/rust-recovery-2026-10-12/deferred-probes.txt`.

The calendar contract now checks the standalone 30-day calendar helper and separately requires refusal/unchanged date from the full native week. Tiny SQLite uses the complete codec fixture rather than the old partial schema.

```powershell
cargo test --offline --manifest-path reports/review-2026-10-12/probes/Cargo.toml --test contracts
cargo test --offline --manifest-path reports/review-2026-10-12/probes/Cargo.toml --test contracts -- --ignored
```

No full-world weeks are run. The nine deferred reproductions use only tiny RAM states and in-process HTTP requests.
