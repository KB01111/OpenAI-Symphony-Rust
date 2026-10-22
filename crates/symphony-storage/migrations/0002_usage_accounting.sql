-- 0002_usage_accounting.sql: durable baselines for resumed provider threads.
--
-- `thread_usage_*` hold the absolute usage of the provider thread (`current_thread_id`) that earlier
-- attempts already accounted for, so a resumed thread's cumulative reports are never charged twice.
-- `usage_estimated` marks token totals that could not be measured exactly. Existing rows keep their
-- totals untouched: NULL means "no baseline recorded" (a legacy task), 0 means "measured exactly".

ALTER TABLE tasks ADD COLUMN thread_usage_thread_id TEXT;
ALTER TABLE tasks ADD COLUMN thread_usage_input_tokens INTEGER;
ALTER TABLE tasks ADD COLUMN thread_usage_output_tokens INTEGER;
ALTER TABLE tasks ADD COLUMN thread_usage_total_tokens INTEGER;
ALTER TABLE tasks ADD COLUMN usage_estimated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE run_attempts ADD COLUMN usage_estimated INTEGER NOT NULL DEFAULT 0;
