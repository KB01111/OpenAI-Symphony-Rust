//! Connection profiles live in `provider_profiles` (migration 0006 adds the model-service columns).

use chrono::{TimeZone, Utc};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use symphony_core::{ModelService, ProfileStatus, ProviderKind, ProviderProfile};
use symphony_storage::Storage;

fn profile(id: &str, service: ModelService) -> ProviderProfile {
    ProviderProfile {
        id: id.into(),
        name: format!("{} profile", service.display_name()),
        provider: ProviderKind::OpenCode,
        model: Some("anthropic/claude-sonnet-5.5".into()),
        command_override: None,
        approval_policy: "never".into(),
        sandbox_mode: "workspace-write".into(),
        credential_ref: Some(service.credential_key().into()),
        is_default: false,
        service: Some(service),
        base_url: None,
        status: ProfileStatus::Configured,
        status_detail: None,
        verified_at: None,
        fallback_profile_id: None,
    }
}

#[tokio::test]
async fn a_connection_profile_round_trips_with_its_test_result() {
    let storage = Storage::connect_in_memory().await.unwrap();
    let mut p = profile("or", ModelService::OpenRouter);
    p.base_url = Some("https://openrouter.ai/api/v1".into());
    p.status = ProfileStatus::Verified;
    p.status_detail = Some("312 models".into());
    p.verified_at = Some(Utc.with_ymd_and_hms(2026, 11, 5, 8, 30, 0).unwrap());
    p.fallback_profile_id = Some("nv".into());
    storage.upsert_provider_profile(&p).await.unwrap();

    assert_eq!(
        storage.get_provider_profile("or").await.unwrap(),
        Some(p.clone())
    );
    assert_eq!(storage.get_provider_profile("missing").await.unwrap(), None);

    // An update replaces the test result.
    p.status = ProfileStatus::Error;
    p.status_detail = Some("the service refused the key".into());
    storage.upsert_provider_profile(&p).await.unwrap();
    let all = storage.list_provider_profiles().await.unwrap();
    assert_eq!(all, vec![p]);
}

#[tokio::test]
async fn deleting_a_profile_clears_it_as_a_fallback() {
    let storage = Storage::connect_in_memory().await.unwrap();
    let mut main = profile("or", ModelService::OpenRouter);
    main.fallback_profile_id = Some("nv".into());
    storage.upsert_provider_profile(&main).await.unwrap();
    storage
        .upsert_provider_profile(&profile("nv", ModelService::NvidiaBuild))
        .await
        .unwrap();

    assert!(storage.delete_provider_profile("nv").await.unwrap());
    assert!(!storage.delete_provider_profile("nv").await.unwrap());
    let left = storage.get_provider_profile("or").await.unwrap().unwrap();
    assert_eq!(left.fallback_profile_id, None);
    assert_eq!(storage.list_provider_profiles().await.unwrap().len(), 1);
}

/// A database at schema 5 keeps its profiles; they read back as untested agent entries.
#[tokio::test]
async fn upgrades_schema_five_and_keeps_existing_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    {
        let options = SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        for sql in [
            include_str!("../migrations/0001_init.sql"),
            include_str!("../migrations/0002_usage_accounting.sql"),
            include_str!("../migrations/0003_task_follow_ups.sql"),
            include_str!("../migrations/0004_tool_calls.sql"),
            include_str!("../migrations/0005_cloud_agent.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, name, applied_at) VALUES
                (1, 'init', 'x'), (2, 'usage_accounting', 'x'), (3, 'task_follow_ups', 'x'),
                (4, 'tool_calls', 'x'), (5, 'cloud_agent', 'x');
             INSERT INTO provider_profiles (id, name, provider, model, command_override,
                approval_policy, sandbox_mode, credential_ref, is_default)
                VALUES ('opencode-default', 'OpenCode CLI', 'opencode', NULL, 'opencode',
                        'never', 'workspace-write', NULL, 1);",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let storage = Storage::connect(&db).await.unwrap();
    let old = storage
        .get_provider_profile("opencode-default")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.provider, ProviderKind::OpenCode);
    assert!(old.is_default);
    assert_eq!(old.service, None);
    assert_eq!(old.status, ProfileStatus::Configured);

    storage
        .upsert_provider_profile(&profile("go", ModelService::OpenCodeGo))
        .await
        .unwrap();
    assert_eq!(storage.list_provider_profiles().await.unwrap().len(), 2);
}
