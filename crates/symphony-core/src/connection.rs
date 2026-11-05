//! Connection profiles: the *model service* a coding agent talks to (OpenRouter, OpenCode Go,
//! NVIDIA Build), separate from the coding agent itself ([`ProviderKind`]).
//!
//! [`resolve_connection`] is the single place that turns a stored [`ProviderProfile`] into what is
//! actually used: model discovery and the connection test call the service with the same base URL
//! and key name, and a run hands OpenCode the same provider id, model and config. A resolved
//! connection never holds the API key; the key stays in the credential vault and reaches the agent
//! only as an environment variable named by [`ResolvedConnection::key_env_name`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;
use utoipa::ToSchema;

use crate::domain::{ProviderKind, ProviderProfile};
use crate::error::{SymphonyError, SymphonyResult};

/// A hosted service that provides models over an OpenAI-compatible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub enum ModelService {
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "opencode_go")]
    OpenCodeGo,
    #[serde(rename = "nvidia_build")]
    NvidiaBuild,
}

impl ModelService {
    pub const ALL: [ModelService; 3] = [Self::OpenRouter, Self::OpenCodeGo, Self::NvidiaBuild];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenCodeGo => "opencode_go",
            Self::NvidiaBuild => "nvidia_build",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::OpenCodeGo => "OpenCode Go",
            Self::NvidiaBuild => "NVIDIA Build",
        }
    }

    /// Where the service is served from unless a profile says otherwise.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::OpenCodeGo => "https://opencode.ai/inference/go",
            Self::NvidiaBuild => "https://integrate.api.nvidia.com/v1",
        }
    }

    /// Hosts a profile may point at. A profile's key is sent to its base URL, so a base URL on
    /// any other host is refused rather than trusted.
    pub fn allowed_hosts(self) -> &'static [&'static str] {
        match self {
            Self::OpenRouter => &["openrouter.ai"],
            Self::OpenCodeGo => &["opencode.ai"],
            Self::NvidiaBuild => &["integrate.api.nvidia.com"],
        }
    }

    /// Name of the key in the credential vault (and in the credential allowlist).
    pub fn credential_key(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OpenCodeGo => "OPENCODE_GO_API_KEY",
            Self::NvidiaBuild => "NVIDIA_API_KEY",
        }
    }

    /// Environment variable OpenCode reads the key from. OpenCode Go shares `OPENCODE_API_KEY`
    /// with OpenCode Zen, so the vault name and the variable differ for that service.
    pub fn key_env_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OpenCodeGo => "OPENCODE_API_KEY",
            Self::NvidiaBuild => "NVIDIA_API_KEY",
        }
    }

    /// Provider id in OpenCode's catalogue: the prefix of a model reference (`provider/model`).
    pub fn opencode_provider_id(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenCodeGo => "opencode-go",
            Self::NvidiaBuild => "nvidia",
        }
    }

    /// Every vault key that belongs to a service.
    pub fn credential_keys() -> [&'static str; 3] {
        Self::ALL.map(Self::credential_key)
    }

    /// The service whose provider id is the prefix of `model` (`openrouter/anthropic/...`).
    pub fn from_model_ref(model: &str) -> Option<Self> {
        let (prefix, _) = model.split_once('/')?;
        Self::ALL
            .into_iter()
            .find(|s| s.opencode_provider_id() == prefix)
    }
}

impl std::str::FromStr for ModelService {
    type Err = SymphonyError;

    fn from_str(value: &str) -> SymphonyResult<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
            .ok_or_else(|| SymphonyError::InvalidConfig(format!("unknown model service `{value}`")))
    }
}

/// What the last connection test found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    /// Saved but not tested yet.
    #[default]
    Configured,
    /// The service accepted the key and listed its models.
    Verified,
    /// The key is accepted (or could not be refused) but the service did not list models.
    Limited,
    /// The test failed: no key, a refused key, or the service was unreachable.
    Error,
}

impl ProfileStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Verified => "verified",
            Self::Limited => "limited",
            Self::Error => "error",
        }
    }

    /// An unknown stored value reads as `configured`, never as a verified connection.
    pub fn from_stored(value: &str) -> Self {
        match value {
            "verified" => Self::Verified,
            "limited" => Self::Limited,
            "error" => Self::Error,
            _ => Self::Configured,
        }
    }
}

const MAX_NAME_CHARS: usize = 80;
const MAX_MODEL_CHARS: usize = 200;

/// The base URL a profile uses: its own, or the service's default; without a trailing slash.
pub fn effective_base_url(profile: &ProviderProfile, service: ModelService) -> String {
    profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| service.default_base_url())
        .trim_end_matches('/')
        .to_string()
}

/// Whether `base_url` is somewhere the service's key may be sent: `https`, an allowed host, the
/// default port, no credentials, query or fragment.
pub fn validate_base_url(service: ModelService, base_url: &str) -> SymphonyResult<()> {
    let bad = |why: &str| {
        SymphonyError::InvalidConfig(format!(
            "base URL for {} {why}; it must be https on {}",
            service.display_name(),
            service.allowed_hosts().join(" or ")
        ))
    };
    let url = Url::parse(base_url).map_err(|_| bad("is not a valid URL"))?;
    if url.scheme() != "https" {
        return Err(bad("is not https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("contains credentials"));
    }
    if url.port().is_some() {
        return Err(bad("sets a port"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(bad("has a query or fragment"));
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if !service.allowed_hosts().contains(&host.as_str()) {
        return Err(bad("is on another host"));
    }
    Ok(())
}

fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.chars().count() <= MAX_MODEL_CHARS
        && model.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | '+')
        })
}

/// Check a profile before it is stored or used.
pub fn validate_profile(profile: &ProviderProfile) -> SymphonyResult<ModelService> {
    let invalid = |msg: String| Err(SymphonyError::InvalidConfig(msg));
    let Some(service) = profile.service else {
        return invalid("a connection profile needs a model service".into());
    };
    if profile.provider != ProviderKind::OpenCode {
        return invalid("a model service connection is used by the OpenCode agent".into());
    }
    let name = profile.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return invalid(format!(
            "the profile name must be 1-{MAX_NAME_CHARS} characters"
        ));
    }
    if profile.id.is_empty()
        || profile.id.len() > 64
        || !profile
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return invalid("the profile id may hold letters, digits, `-` and `_` (at most 64)".into());
    }
    validate_base_url(service, &effective_base_url(profile, service))?;
    if let Some(model) = profile.model.as_deref() {
        if !valid_model(model) {
            return invalid("the default model contains characters a model id does not".into());
        }
    }
    if let Some(reference) = profile.credential_ref.as_deref() {
        if reference != service.credential_key() {
            return invalid(format!(
                "{} uses the credential {}",
                service.display_name(),
                service.credential_key()
            ));
        }
    }
    if profile.fallback_profile_id.as_deref() == Some(profile.id.as_str()) {
        return invalid("a profile cannot be its own fallback".into());
    }
    Ok(service)
}

/// A profile as used by discovery, the connection test and a run. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConnection {
    pub profile_id: String,
    pub service: ModelService,
    pub base_url: String,
    /// Model id after the provider prefix, as the service names it.
    pub model_id: Option<String>,
}

impl ResolvedConnection {
    pub fn credential_key(&self) -> &'static str {
        self.service.credential_key()
    }

    pub fn key_env_name(&self) -> &'static str {
        self.service.key_env_name()
    }

    /// `provider/model` for OpenCode, which splits it at the first `/`.
    pub fn qualified_model(&self) -> Option<String> {
        self.model_id
            .as_deref()
            .map(|m| format!("{}/{m}", self.service.opencode_provider_id()))
    }

    /// Where the service lists its models.
    pub fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    /// Inline config for `OPENCODE_CONFIG_CONTENT`. The key is a `{env:…}` reference, never a value.
    pub fn opencode_config(&self) -> Value {
        json!({
            "provider": {
                self.service.opencode_provider_id(): {
                    "options": {
                        "baseURL": self.base_url,
                        "apiKey": format!("{{env:{}}}", self.key_env_name()),
                    }
                }
            }
        })
    }
}

/// Resolve `profile` for a run of `requested_model` (a `provider/model` reference or a bare model
/// id), falling back to the profile's default model.
pub fn resolve_connection(
    profile: &ProviderProfile,
    requested_model: Option<&str>,
) -> SymphonyResult<ResolvedConnection> {
    let service = validate_profile(profile)?;
    let prefix = format!("{}/", service.opencode_provider_id());
    let model_id = requested_model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .or(profile.model.as_deref())
        .map(|m| m.strip_prefix(&prefix).unwrap_or(m).to_string());
    if let Some(model) = &model_id {
        if !valid_model(model) {
            return Err(SymphonyError::InvalidConfig(
                "the model contains characters a model id does not".into(),
            ));
        }
    }
    Ok(ResolvedConnection {
        profile_id: profile.id.clone(),
        service,
        base_url: effective_base_url(profile, service),
        model_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(service: ModelService) -> ProviderProfile {
        ProviderProfile {
            id: "p1".into(),
            name: "Main".into(),
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

    #[test]
    fn every_service_has_the_documented_preset() {
        let presets = [
            (
                ModelService::OpenRouter,
                "https://openrouter.ai/api/v1",
                "openrouter",
            ),
            (
                ModelService::OpenCodeGo,
                "https://opencode.ai/inference/go",
                "opencode-go",
            ),
            (
                ModelService::NvidiaBuild,
                "https://integrate.api.nvidia.com/v1",
                "nvidia",
            ),
        ];
        for (service, url, provider) in presets {
            assert_eq!(service.default_base_url(), url);
            assert_eq!(service.opencode_provider_id(), provider);
            validate_base_url(service, url).unwrap();
            let resolved = resolve_connection(&profile(service), None).unwrap();
            assert_eq!(resolved.base_url, url);
            assert_eq!(
                resolved.qualified_model().unwrap(),
                format!("{provider}/anthropic/claude-sonnet-5.5")
            );
            assert_eq!(resolved.models_url(), format!("{url}/models"));
        }
        assert_eq!(ModelService::OpenCodeGo.key_env_name(), "OPENCODE_API_KEY");
        assert_eq!(
            ModelService::OpenCodeGo.credential_key(),
            "OPENCODE_GO_API_KEY"
        );
    }

    #[test]
    fn the_config_references_the_key_and_never_holds_it() {
        let resolved = resolve_connection(&profile(ModelService::OpenRouter), None).unwrap();
        let config = resolved.opencode_config();
        assert_eq!(
            config["provider"]["openrouter"]["options"]["apiKey"],
            "{env:OPENROUTER_API_KEY}"
        );
        assert_eq!(
            config["provider"]["openrouter"]["options"]["baseURL"],
            "https://openrouter.ai/api/v1"
        );
    }

    #[test]
    fn a_requested_model_wins_and_loses_its_provider_prefix() {
        let p = profile(ModelService::OpenRouter);
        let by_ref = resolve_connection(&p, Some("openrouter/openai/gpt-5.5")).unwrap();
        assert_eq!(by_ref.model_id.as_deref(), Some("openai/gpt-5.5"));
        let bare = resolve_connection(&p, Some("openai/gpt-5.5")).unwrap();
        assert_eq!(bare.model_id.as_deref(), Some("openai/gpt-5.5"));
        let default = resolve_connection(&p, None).unwrap();
        assert_eq!(
            default.model_id.as_deref(),
            Some("anthropic/claude-sonnet-5.5")
        );
        assert_eq!(
            ModelService::from_model_ref("nvidia/meta/llama"),
            Some(ModelService::NvidiaBuild)
        );
        assert_eq!(ModelService::from_model_ref("anthropic/claude"), None);
    }

    #[test]
    fn a_base_url_that_could_leak_the_key_is_refused() {
        let svc = ModelService::OpenRouter;
        for bad in [
            "http://openrouter.ai/api/v1",
            "https://openrouter.ai.evil.com/api/v1",
            "https://evil.com/openrouter.ai",
            "https://user:pw@openrouter.ai/api/v1",
            "https://openrouter.ai:8443/api/v1",
            "https://openrouter.ai/api/v1?x=1",
            "https://sub.openrouter.ai/api/v1",
            "https://integrate.api.nvidia.com/v1",
            "not a url",
            "",
        ] {
            assert!(validate_base_url(svc, bad).is_err(), "{bad}");
        }
        validate_base_url(svc, "https://OPENROUTER.ai/api/v1/").unwrap();
        let mut p = profile(svc);
        p.base_url = Some("https://evil.com/v1".into());
        assert!(resolve_connection(&p, None).is_err());
        p.base_url = Some("https://openrouter.ai/api/v1/".into());
        assert_eq!(
            resolve_connection(&p, None).unwrap().base_url,
            "https://openrouter.ai/api/v1"
        );
    }

    #[test]
    fn profile_fields_are_validated() {
        let mut p = profile(ModelService::NvidiaBuild);
        validate_profile(&p).unwrap();
        p.name = "  ".into();
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.credential_ref = Some("OPENROUTER_API_KEY".into());
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.model = Some("bad model; rm".into());
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.id = "../x".into();
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.fallback_profile_id = Some("p1".into());
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.provider = ProviderKind::Codex;
        assert!(validate_profile(&p).is_err());
        let mut p = profile(ModelService::NvidiaBuild);
        p.service = None;
        assert!(validate_profile(&p).is_err());
    }

    #[test]
    fn an_unknown_stored_status_is_never_verified() {
        assert_eq!(
            ProfileStatus::from_stored("verified"),
            ProfileStatus::Verified
        );
        assert_eq!(
            ProfileStatus::from_stored("bogus"),
            ProfileStatus::Configured
        );
        assert_eq!(
            "openrouter".parse::<ModelService>().unwrap(),
            ModelService::OpenRouter
        );
        assert!("nope".parse::<ModelService>().is_err());
    }
}
