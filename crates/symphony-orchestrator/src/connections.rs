//! Connection profiles: the model service (OpenRouter, OpenCode Go, NVIDIA Build) an OpenCode run
//! talks to. Discovery, the connection test and the run all start from
//! [`symphony_core::resolve_connection`], so what was tested is what runs.

use super::*;
use symphony_agents::connection_probe::{probe_models, ProbeResult};
use symphony_core::{
    resolve_connection, select_profile, validate_profile, ConnectionProfileInput, ModelService,
    ProfileStatus, ProviderProfile, ResolvedConnection, Selection,
};

/// Asks a model service which models a key can use.
#[async_trait]
pub trait ModelLister: Send + Sync {
    async fn list(&self, connection: &ResolvedConnection, api_key: &str) -> ProbeResult;
}

/// The real thing: `GET {base_url}/models` with the key.
pub struct HttpModelLister;

#[async_trait]
impl ModelLister for HttpModelLister {
    async fn list(&self, connection: &ResolvedConnection, api_key: &str) -> ProbeResult {
        probe_models(connection, api_key).await
    }
}

/// A profile after a connection test, with the models the service listed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConnectionTestResult {
    pub profile: ProviderProfile,
    pub models: Vec<String>,
}

/// The models a profile's service lists right now (the stored status is not changed).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConnectionModels {
    pub status: ProfileStatus,
    pub detail: String,
    pub models: Vec<String>,
}

/// What a run through a profile needs: the environment for the agent process and the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConnection {
    pub env: HashMap<String, String>,
    /// `provider/model` for OpenCode; `None` leaves the choice to OpenCode.
    pub model: Option<String>,
    pub profile_name: String,
    pub fell_back: bool,
}

fn not_found(id: &str) -> SymphonyError {
    SymphonyError::NotFound(format!("connection profile {id}"))
}

impl SymphonyOrchestrator {
    #[doc(hidden)]
    pub fn set_model_lister(&self, lister: Arc<dyn ModelLister>) {
        *self.model_lister.write().unwrap_or_else(|e| e.into_inner()) = lister;
    }

    fn model_lister(&self) -> Arc<dyn ModelLister> {
        self.model_lister
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Connection profiles (the built-in agent entries are not among them).
    pub async fn connection_profiles(&self) -> SymphonyResult<Vec<ProviderProfile>> {
        Ok(self
            .storage
            .list_provider_profiles()
            .await?
            .into_iter()
            .filter(|p| p.service.is_some())
            .collect())
    }

    /// Create (`id` is `None`) or change a profile. A change of service or base URL clears the last
    /// test result; the key is never part of a profile.
    pub async fn save_connection_profile(
        &self,
        id: Option<String>,
        input: ConnectionProfileInput,
    ) -> SymphonyResult<ProviderProfile> {
        let _changes = self.connections_lock.lock().await;
        let existing = match &id {
            Some(id) => Some(
                self.storage
                    .get_provider_profile(id)
                    .await?
                    .filter(|p| p.service.is_some())
                    .ok_or_else(|| not_found(id))?,
            ),
            None => None,
        };
        let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut profile = ProviderProfile {
            id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: input.name.trim().to_string(),
            provider: ProviderKind::OpenCode,
            model: clean(input.model),
            command_override: None,
            approval_policy: "never".into(),
            sandbox_mode: "workspace-write".into(),
            credential_ref: Some(input.service.credential_key().into()),
            is_default: input.is_default,
            service: Some(input.service),
            base_url: clean(input.base_url),
            status: ProfileStatus::Configured,
            status_detail: None,
            verified_at: None,
            fallback_profile_id: clean(input.fallback_profile_id),
        };
        validate_profile(&profile)?;
        if let Some(fallback) = &profile.fallback_profile_id {
            let target = self.storage.get_provider_profile(fallback).await?;
            if !target.is_some_and(|t| t.service.is_some()) {
                return Err(SymphonyError::InvalidConfig(
                    "the fallback must be another connection profile".into(),
                ));
            }
        }
        if let Some(old) = existing {
            let same_target = old.service == profile.service
                && symphony_core::effective_base_url(&old, input.service)
                    == symphony_core::effective_base_url(&profile, input.service);
            if same_target {
                profile.status = old.status;
                profile.status_detail = old.status_detail;
                profile.verified_at = old.verified_at;
            }
        }
        self.storage.upsert_provider_profile(&profile).await?;
        if profile.is_default {
            for other in self.connection_profiles().await? {
                if other.id != profile.id && other.is_default {
                    self.storage
                        .upsert_provider_profile(&ProviderProfile {
                            is_default: false,
                            ..other
                        })
                        .await?;
                }
            }
        }
        self.invalidate_provider_cache().await;
        Ok(profile)
    }

    pub async fn delete_connection_profile(&self, id: &str) -> SymphonyResult<()> {
        let _changes = self.connections_lock.lock().await;
        let existing = self.storage.get_provider_profile(id).await?;
        if !existing.is_some_and(|p| p.service.is_some()) {
            return Err(not_found(id));
        }
        self.storage.delete_provider_profile(id).await?;
        self.invalidate_provider_cache().await;
        Ok(())
    }

    async fn service_profile(&self, id: &str) -> SymphonyResult<ProviderProfile> {
        self.storage
            .get_provider_profile(id)
            .await?
            .filter(|p| p.service.is_some())
            .ok_or_else(|| not_found(id))
    }

    /// Ask the profile's service for its models with the profile's key.
    async fn probe_profile(&self, profile: &ProviderProfile) -> SymphonyResult<ProbeResult> {
        let connection = resolve_connection(profile, None)?;
        let Some(key) = self
            .secret_store
            .get_vault_secret(connection.credential_key())
            .filter(|k| !k.trim().is_empty())
        else {
            return Ok(ProbeResult {
                status: ProfileStatus::Error,
                detail: format!("no {} stored", connection.credential_key()),
                models: Vec::new(),
            });
        };
        let mut result = self.model_lister().list(&connection, &key).await;
        // Whatever the service or an error path put in the text, the key never leaves here.
        result.detail = redact_secrets(&result.detail, std::slice::from_ref(&key));
        result.models.retain(|model| !model.contains(key.as_str()));
        Ok(result)
    }

    /// Test a profile's connection and store the outcome.
    ///
    /// The result is recorded only if the profile is still the one that was probed and its key was
    /// not changed meanwhile; otherwise the current profile is returned with the models found.
    pub async fn test_connection_profile(&self, id: &str) -> SymphonyResult<ConnectionTestResult> {
        use std::sync::atomic::Ordering;
        let probed = self.service_profile(id).await?;
        let epoch = self.credential_epoch.load(Ordering::SeqCst);
        let result = self.probe_profile(&probed).await?;

        let _changes = self.connections_lock.lock().await;
        let mut profile = self.service_profile(id).await?;
        let target = |p: &ProviderProfile| {
            p.service
                .map(|service| (service, symphony_core::effective_base_url(p, service)))
        };
        if target(&profile) == target(&probed)
            && self.credential_epoch.load(Ordering::SeqCst) == epoch
        {
            profile.status = result.status;
            profile.status_detail = Some(result.detail);
            profile.verified_at = (result.status == ProfileStatus::Verified).then(Utc::now);
            self.storage.upsert_provider_profile(&profile).await?;
            self.invalidate_provider_cache().await;
        }
        Ok(ConnectionTestResult {
            profile,
            models: result.models,
        })
    }

    /// The models the profile's service lists, without touching the stored status.
    pub async fn connection_profile_models(&self, id: &str) -> SymphonyResult<ConnectionModels> {
        let profile = self.service_profile(id).await?;
        let result = self.probe_profile(&profile).await?;
        Ok(ConnectionModels {
            status: result.status,
            detail: result.detail,
            models: result.models,
        })
    }

    /// A key was stored, replaced or removed: what earlier tests found no longer holds.
    pub async fn connection_credential_changed(&self, key: &str) -> SymphonyResult<()> {
        let Some(service) = ModelService::ALL
            .into_iter()
            .find(|s| s.credential_key() == key)
        else {
            return Ok(());
        };
        self.credential_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _changes = self.connections_lock.lock().await;
        for profile in self.connection_profiles().await? {
            if profile.service == Some(service) && profile.status != ProfileStatus::Configured {
                self.storage
                    .upsert_provider_profile(&ProviderProfile {
                        status: ProfileStatus::Configured,
                        status_detail: None,
                        verified_at: None,
                        ..profile
                    })
                    .await?;
            }
        }
        self.invalidate_provider_cache().await;
        Ok(())
    }

    /// The connection an OpenCode run uses for `requested_model`, if a profile applies. The key
    /// reaches the agent only as the environment variable named by the resolved connection.
    pub async fn resolve_run_connection(
        &self,
        requested_model: Option<&str>,
    ) -> SymphonyResult<Option<RunConnection>> {
        let profiles = self.connection_profiles().await?;
        let has_key = |service: ModelService| {
            self.secret_store
                .get_vault_secret(service.credential_key())
                .is_some_and(|k| !k.trim().is_empty())
        };
        let (profile, fell_back) =
            match select_profile(&profiles, requested_model, has_key) {
                Selection::NoProfile => return Ok(None),
                Selection::MissingKey { profile } => {
                    return Err(SymphonyError::InvalidConfig(format!(
                    "Connection profile \"{}\" has no API key: add {} in Settings → Connections.",
                    profile.name,
                    profile.service.map(|s| s.credential_key()).unwrap_or_default()
                )))
                }
                Selection::Use { profile, fell_back } => (profile, fell_back),
            };
        // A fallback runs on its own default model: the requested one names the other service.
        let connection =
            resolve_connection(profile, if fell_back { None } else { requested_model })?;
        // Read again: the key may have been removed since the profile was selected.
        let key = self
            .secret_store
            .get_vault_secret(connection.credential_key())
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                SymphonyError::InvalidConfig(format!(
                    "Connection profile \"{}\" has no API key: add {} in Settings → Connections.",
                    profile.name,
                    connection.credential_key()
                ))
            })?;
        let config = connection.opencode_config().to_string();
        Ok(Some(RunConnection {
            env: HashMap::from([
                (connection.key_env_name().to_string(), key),
                ("OPENCODE_CONFIG_CONTENT".to_string(), config),
            ]),
            model: connection.qualified_model(),
            profile_name: profile.name.clone(),
            fell_back,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    const KEY: &str = "sk-or-test-1234567890abcdef";

    /// Answers like a service and remembers what it was asked.
    struct FakeLister {
        result: ProbeResult,
        asked: StdMutex<Vec<(ResolvedConnection, String)>>,
    }

    impl FakeLister {
        fn new(status: ProfileStatus, detail: &str, models: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                result: ProbeResult {
                    status,
                    detail: detail.into(),
                    models: models.iter().map(|m| m.to_string()).collect(),
                },
                asked: StdMutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ModelLister for FakeLister {
        async fn list(&self, connection: &ResolvedConnection, api_key: &str) -> ProbeResult {
            self.asked
                .lock()
                .unwrap()
                .push((connection.clone(), api_key.to_string()));
            self.result.clone()
        }
    }

    async fn orchestrator() -> (SymphonyOrchestrator, TempDir) {
        let temp = TempDir::new().unwrap();
        let orch = SymphonyOrchestrator::new(
            Storage::connect_in_memory().await.unwrap(),
            None,
            SecretStore::new(&temp.path().join("vault")).unwrap(),
        )
        .await
        .unwrap();
        (orch, temp)
    }

    fn input(name: &str, service: ModelService) -> ConnectionProfileInput {
        ConnectionProfileInput {
            name: name.into(),
            service,
            base_url: None,
            model: Some("anthropic/claude-sonnet-5.5".into()),
            fallback_profile_id: None,
            is_default: true,
        }
    }

    #[tokio::test]
    async fn the_test_and_the_run_use_the_same_resolved_profile() {
        let (orch, _dir) = orchestrator().await;
        let lister = FakeLister::new(ProfileStatus::Verified, "2 models", &["a/b", "c/d"]);
        orch.set_model_lister(lister.clone());
        let profile = orch
            .save_connection_profile(None, input("Main", ModelService::OpenRouter))
            .await
            .unwrap();
        assert_eq!(profile.status, ProfileStatus::Configured);
        assert_eq!(
            profile.credential_ref.as_deref(),
            Some("OPENROUTER_API_KEY")
        );
        orch.secret_store()
            .set_secret("OPENROUTER_API_KEY", KEY)
            .unwrap();

        let tested = orch.test_connection_profile(&profile.id).await.unwrap();
        assert_eq!(tested.profile.status, ProfileStatus::Verified);
        assert!(tested.profile.verified_at.is_some());
        assert_eq!(tested.models, ["a/b", "c/d"]);
        // The stored profile carries the result.
        let stored = orch.connection_profiles().await.unwrap();
        assert_eq!(stored[0].status, ProfileStatus::Verified);

        let run = orch.resolve_run_connection(None).await.unwrap().unwrap();
        let (tested_connection, tested_key) = lister.asked.lock().unwrap()[0].clone();
        assert_eq!(tested_key, KEY);
        // The run is handed the config of exactly the connection that was tested: the same service
        // and key variable, and on the preset the endpoint stays with OpenCode's own provider.
        assert_eq!(tested_connection.service, ModelService::OpenRouter);
        assert_eq!(tested_connection.base_url, "https://openrouter.ai/api/v1");
        let config: Value = serde_json::from_str(&run.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
        let options = &config["provider"]["openrouter"]["options"];
        assert_eq!(options["apiKey"], "{env:OPENROUTER_API_KEY}");
        assert!(options.get("baseURL").is_none());
        assert_eq!(run.env["OPENROUTER_API_KEY"], KEY);
        assert_eq!(
            run.model.as_deref(),
            Some("openrouter/anthropic/claude-sonnet-5.5")
        );
        // The key is an environment variable, never part of the config text.
        assert!(!run.env["OPENCODE_CONFIG_CONTENT"].contains(KEY));
        assert!(!run.fell_back);
    }

    #[tokio::test]
    async fn a_missing_key_is_reported_by_the_test_and_blocks_the_run() {
        let (orch, _dir) = orchestrator().await;
        let lister = FakeLister::new(ProfileStatus::Verified, "ok", &["m"]);
        orch.set_model_lister(lister.clone());
        let profile = orch
            .save_connection_profile(None, input("Main", ModelService::NvidiaBuild))
            .await
            .unwrap();
        let tested = orch.test_connection_profile(&profile.id).await.unwrap();
        assert_eq!(tested.profile.status, ProfileStatus::Error);
        assert!(tested
            .profile
            .status_detail
            .unwrap()
            .contains("NVIDIA_API_KEY"));
        assert!(
            lister.asked.lock().unwrap().is_empty(),
            "no key, no request"
        );

        let err = orch
            .resolve_run_connection(None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Main") && err.contains("NVIDIA_API_KEY"),
            "{err}"
        );
        // A model of another provider is left to OpenCode.
        assert!(orch
            .resolve_run_connection(Some("anthropic/claude-sonnet-5.5"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn the_fallback_profile_runs_on_its_own_default_model() {
        let (orch, _dir) = orchestrator().await;
        let backup = orch
            .save_connection_profile(
                None,
                ConnectionProfileInput {
                    model: Some("meta/llama-4".into()),
                    is_default: false,
                    ..input("Backup", ModelService::NvidiaBuild)
                },
            )
            .await
            .unwrap();
        orch.save_connection_profile(
            None,
            ConnectionProfileInput {
                fallback_profile_id: Some(backup.id.clone()),
                ..input("Main", ModelService::OpenRouter)
            },
        )
        .await
        .unwrap();
        orch.secret_store()
            .set_secret("NVIDIA_API_KEY", "nvapi-test-0123456789")
            .unwrap();

        let run = orch
            .resolve_run_connection(Some("openrouter/openai/gpt-5.5"))
            .await
            .unwrap()
            .unwrap();
        assert!(run.fell_back);
        assert_eq!(run.profile_name, "Backup");
        assert_eq!(run.model.as_deref(), Some("nvidia/meta/llama-4"));
        assert!(run.env.contains_key("NVIDIA_API_KEY"));
        assert!(!run.env.contains_key("OPENROUTER_API_KEY"));
    }

    #[tokio::test]
    async fn changing_the_target_or_the_key_clears_the_last_test() {
        let (orch, _dir) = orchestrator().await;
        orch.set_model_lister(FakeLister::new(ProfileStatus::Verified, "1 models", &["m"]));
        let profile = orch
            .save_connection_profile(None, input("Main", ModelService::OpenRouter))
            .await
            .unwrap();
        orch.secret_store()
            .set_secret("OPENROUTER_API_KEY", KEY)
            .unwrap();
        orch.test_connection_profile(&profile.id).await.unwrap();

        // Renaming or picking another model keeps the result.
        let renamed = orch
            .save_connection_profile(
                Some(profile.id.clone()),
                ConnectionProfileInput {
                    model: Some("openai/gpt-5.5".into()),
                    ..input("Renamed", ModelService::OpenRouter)
                },
            )
            .await
            .unwrap();
        assert_eq!(renamed.status, ProfileStatus::Verified);

        // A new base URL (still on the service's host) does not.
        let moved = orch
            .save_connection_profile(
                Some(profile.id.clone()),
                ConnectionProfileInput {
                    base_url: Some("https://openrouter.ai/api/v1/other".into()),
                    ..input("Renamed", ModelService::OpenRouter)
                },
            )
            .await
            .unwrap();
        assert_eq!(moved.status, ProfileStatus::Configured);

        orch.test_connection_profile(&profile.id).await.unwrap();
        orch.connection_credential_changed("OPENROUTER_API_KEY")
            .await
            .unwrap();
        let after = orch.connection_profiles().await.unwrap();
        assert_eq!(after[0].status, ProfileStatus::Configured);
        assert_eq!(after[0].verified_at, None);
    }

    #[tokio::test]
    async fn profiles_are_validated_and_only_one_is_the_default() {
        let (orch, _dir) = orchestrator().await;
        let bad_url = orch
            .save_connection_profile(
                None,
                ConnectionProfileInput {
                    base_url: Some("https://evil.example.com/v1".into()),
                    ..input("X", ModelService::OpenRouter)
                },
            )
            .await;
        assert!(matches!(bad_url, Err(SymphonyError::InvalidConfig(_))));
        let bad_fallback = orch
            .save_connection_profile(
                None,
                ConnectionProfileInput {
                    fallback_profile_id: Some("missing".into()),
                    ..input("X", ModelService::OpenRouter)
                },
            )
            .await;
        assert!(matches!(bad_fallback, Err(SymphonyError::InvalidConfig(_))));
        assert!(matches!(
            orch.test_connection_profile("missing").await,
            Err(SymphonyError::NotFound(_))
        ));

        let first = orch
            .save_connection_profile(None, input("One", ModelService::OpenRouter))
            .await
            .unwrap();
        let second = orch
            .save_connection_profile(None, input("Two", ModelService::NvidiaBuild))
            .await
            .unwrap();
        let defaults: Vec<_> = orch
            .connection_profiles()
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.is_default)
            .map(|p| p.id)
            .collect();
        assert_eq!(defaults, std::slice::from_ref(&second.id));

        orch.delete_connection_profile(&first.id).await.unwrap();
        assert!(matches!(
            orch.delete_connection_profile(&first.id).await,
            Err(SymphonyError::NotFound(_))
        ));
    }

    /// Waits for the test to say go, so a change can land while the probe is "on the network".
    struct GatedLister {
        started: tokio::sync::Notify,
        go: tokio::sync::Notify,
        models: Vec<String>,
    }

    #[async_trait]
    impl ModelLister for GatedLister {
        async fn list(&self, _: &ResolvedConnection, _: &str) -> ProbeResult {
            self.started.notify_one();
            self.go.notified().await;
            ProbeResult {
                status: ProfileStatus::Verified,
                detail: "ok".into(),
                models: self.models.clone(),
            }
        }
    }

    #[tokio::test]
    async fn a_test_that_overlaps_a_key_change_or_a_delete_does_not_record_its_result() {
        let (orch, _dir) = orchestrator().await;
        let gate = Arc::new(GatedLister {
            started: tokio::sync::Notify::new(),
            go: tokio::sync::Notify::new(),
            models: vec!["m/1".into(), format!("leaks/{KEY}")],
        });
        orch.set_model_lister(gate.clone());
        let profile = orch
            .save_connection_profile(None, input("Main", ModelService::OpenRouter))
            .await
            .unwrap();
        orch.secret_store()
            .set_secret("OPENROUTER_API_KEY", KEY)
            .unwrap();

        // The key is replaced while the old key is being tested: the stale result is not recorded.
        let running = tokio::spawn({
            let (orch, id) = (orch.clone(), profile.id.clone());
            async move { orch.test_connection_profile(&id).await }
        });
        gate.started.notified().await;
        orch.secret_store()
            .set_secret("OPENROUTER_API_KEY", "sk-or-replaced-0123456789")
            .unwrap();
        orch.connection_credential_changed("OPENROUTER_API_KEY")
            .await
            .unwrap();
        gate.go.notify_one();
        let result = running.await.unwrap().unwrap();
        assert_eq!(result.profile.status, ProfileStatus::Configured);
        assert_eq!(
            orch.connection_profiles().await.unwrap()[0].status,
            ProfileStatus::Configured
        );
        // A service that echoes the key in a model id does not get it past the API.
        assert_eq!(result.models, ["m/1"]);

        // The profile is deleted while it is being tested: it does not come back.
        let running = tokio::spawn({
            let (orch, id) = (orch.clone(), profile.id.clone());
            async move { orch.test_connection_profile(&id).await }
        });
        gate.started.notified().await;
        orch.delete_connection_profile(&profile.id).await.unwrap();
        gate.go.notify_one();
        assert!(matches!(
            running.await.unwrap(),
            Err(SymphonyError::NotFound(_))
        ));
        assert!(orch.connection_profiles().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_saves_leave_exactly_one_default() {
        let (orch, _dir) = orchestrator().await;
        let saves = (0..8).map(|n| {
            let orch = orch.clone();
            async move {
                orch.save_connection_profile(
                    None,
                    input(&format!("P{n}"), ModelService::OpenRouter),
                )
                .await
            }
        });
        for saved in futures::future::join_all(saves).await {
            saved.unwrap();
        }
        let defaults = orch
            .connection_profiles()
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.is_default)
            .count();
        assert_eq!(defaults, 1);
    }
}
