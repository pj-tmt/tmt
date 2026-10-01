-- Any approved host driver's name in the host columns (#570). SQLite cannot
-- alter a CHECK, so bindings, request_attempts, request_responses and
-- host_servers are rebuilt from their own stored definitions with only the
-- host rule widened to the host-name grammar, rows copied unchanged, and
-- their indexes and triggers replayed verbatim. The rebuild is code, not
-- SQL: see storage/migrations/host_names.rs, which first refuses a source
-- that differs from what migrations 1 through 40 produce.
SELECT 1;
