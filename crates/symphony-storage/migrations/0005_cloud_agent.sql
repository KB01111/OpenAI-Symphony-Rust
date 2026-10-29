-- 0005_cloud_agent.sql: which agent runs in the sandbox of a CodeSandbox cloud task.
--
-- NULL means "use the default" (settings, then WORKFLOW.md, then mini-swe-agent). Only read for
-- tasks whose provider is `codesandbox`.
ALTER TABLE tasks ADD COLUMN cloud_agent TEXT;
