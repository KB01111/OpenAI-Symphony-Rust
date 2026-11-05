-- 0006_connection_profiles.sql: connection profiles (a model service a coding agent talks to).
--
-- Built-in agent entries keep NULL `service` and `base_url`. `status` starts as `configured` (saved,
-- not tested); the others are written by the connection test. The API key is not stored here: the
-- credential vault holds it under the name in `credential_ref`.
ALTER TABLE provider_profiles ADD COLUMN service TEXT;
ALTER TABLE provider_profiles ADD COLUMN base_url TEXT;
ALTER TABLE provider_profiles ADD COLUMN status TEXT NOT NULL DEFAULT 'configured';
ALTER TABLE provider_profiles ADD COLUMN status_detail TEXT;
ALTER TABLE provider_profiles ADD COLUMN verified_at TEXT;
ALTER TABLE provider_profiles ADD COLUMN fallback_profile_id TEXT;
