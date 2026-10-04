# Completed-request fixture provenance

`completed-requests.json` uses the public `ls --json` resume shape verified by
core PR #886. The normalized Claude/Codex counter values are copied from the
expectations in `typescript/test/e2e/usage-hooks.e2e.test.ts`, which processes the
redacted real provider line fixtures owned by the core runtime. No provider
files or user state are read. IDs/epochs/session, sequence/observed timestamps
and monotonic receipt times are constructed for this deterministic timeline;
this is a recorded-shape scenario, not a claimed live timing trace. Expected
totals independently use the complete input+output delta 5,421,606, excluding
cached input already included in input, without dividing by elapsed time, and expire
exactly at the 60-second bucket boundary.
