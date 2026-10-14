# Rust diagnostic run, 2026-10-14
Scope changed by user: diagnosis only. No optimization, product source changes, rule changes or active-world advancement.
Existing native runner: load -> initial audit -> initial checkpoint -> week summary -> prepare (ordered economic phases) -> metrics -> apply -> week report -> final checkpoint.
Release binary built with cargo build --release --locked --offline -p kingmaker-cli; build result recorded separately.
Input: transaction-consistent SQLite backup of runtime-selected world, separate diagnostic runtime file. Input/executable/source SHA256 recorded. Snapshot creation and hashing warm filesystem cache; this is process-cold, not disk-cold.
Run: one full native week, workers=1, profile enabled, external Job Object private-memory cap12288MiB, hard timeout1800s. No repeated identical timeout. Monitor peak sampled100ms; supplementary process CPU/memory samples1s.
WPR CPU recording failed0xc5585011. Instrumentation timings/counters are not CPU stack samples. No claim about function-level CPU attribution without narrower evidence.
Report observed durations, completed/unfinished scopes, counters, memory, success/failure, uncertainty and next diagnostic experiments. Do not infer speedup by comparing this run to old partial profiles. Profile overhead is unmeasured unless an equivalent unprofiled run completes.
