//! Local provider credential handles and capability registry for Cowork.
//!
//! This crate deliberately separates secret material from provider snapshots.
//! Prompt-visible exports receive opaque handles, credential state, and routing
//! metadata; raw provider keys only pass through vault store/retrieve APIs.

use cowork_protocol::{Provider, ProviderCredentialStatus};
pub use cowork_protocol::{
    ProviderAuthMethod, ProviderCapability, ProviderCost, ProviderCredentialState, ProviderKind,
    ProviderRateLimits, ProviderRoutingMetadata, ProviderStatus, ProviderToolSupport,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

const REDACTED: &str = "[REDACTED]";
const PROVIDER_FAMILY_METADATA_KEY: &str = "providerFamily";
const BASE_URL_METADATA_KEY: &str = "baseUrl";
const REQUEST_URL_METADATA_KEY: &str = "requestUrl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultBackend {
    Keychain,
    EncryptedFile,
    MemoryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultBackendStatus {
    pub backend: VaultBackend,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

impl VaultBackendStatus {
    pub fn available(backend: VaultBackend) -> Self {
        Self {
            backend,
            available: true,
            blocked_reason: None,
        }
    }

    pub fn blocked(backend: VaultBackend, reason: impl Into<String>) -> Self {
        Self {
            backend,
            available: false,
            blocked_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialHandle(String);

impl CredentialHandle {
    pub fn new(handle: impl Into<String>) -> Result<Self, CredentialVaultError> {
        let handle = handle.into();
        if handle.trim().is_empty()
            || handle.chars().any(char::is_whitespace)
            || handle.chars().any(char::is_control)
        {
            return Err(CredentialVaultError::InvalidHandle);
        }
        Ok(Self(handle))
    }

    pub fn for_provider(provider_id: &str, slot: &str) -> Result<Self, CredentialVaultError> {
        validate_token(provider_id)?;
        validate_token(slot)?;
        Self::new(format!("cowork-vault://providers/{provider_id}/{slot}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CredentialHandle")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for CredentialHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CredentialSecret(String);

impl CredentialSecret {
    pub fn new(secret: impl Into<String>) -> Result<Self, CredentialVaultError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(CredentialVaultError::EmptySecret);
        }
        if secret.trim().is_empty() || secret.chars().any(char::is_control) {
            return Err(CredentialVaultError::InvalidSecret);
        }
        Ok(Self(secret))
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CredentialSecret")
            .field(&REDACTED)
            .finish()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CredentialVaultError {
    #[error("credential vault backend unavailable")]
    BackendUnavailable { backends: Vec<VaultBackendStatus> },
    #[error("credential handle is invalid")]
    InvalidHandle,
    #[error("credential secret is empty")]
    EmptySecret,
    #[error("credential secret is invalid")]
    InvalidSecret,
    #[error("credential handle was not found: {handle}")]
    MissingCredential { handle: CredentialHandle },
}

#[derive(Debug, Clone)]
pub struct LocalCredentialVault {
    backend: VaultBackend,
    secrets: BTreeMap<CredentialHandle, CredentialSecret>,
}

impl LocalCredentialVault {
    pub fn new(backends: Vec<VaultBackendStatus>) -> Result<Self, CredentialVaultError> {
        let backend = backends
            .iter()
            .find(|backend| backend.available)
            .map(|backend| backend.backend)
            .ok_or_else(|| CredentialVaultError::BackendUnavailable { backends })?;
        Ok(Self {
            backend,
            secrets: BTreeMap::new(),
        })
    }

    pub fn memory_only() -> Self {
        Self {
            backend: VaultBackend::MemoryOnly,
            secrets: BTreeMap::new(),
        }
    }

    pub fn backend(&self) -> VaultBackend {
        self.backend
    }

    pub fn store(
        &mut self,
        handle: CredentialHandle,
        secret: CredentialSecret,
    ) -> CredentialHandle {
        self.secrets.insert(handle.clone(), secret);
        handle
    }

    pub fn retrieve(
        &self,
        handle: &CredentialHandle,
    ) -> Result<&CredentialSecret, CredentialVaultError> {
        self.secrets
            .get(handle)
            .ok_or_else(|| CredentialVaultError::MissingCredential {
                handle: handle.clone(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegistration {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub status: ProviderStatus,
    pub selected_model: Option<String>,
    pub capabilities: Vec<ProviderCapability>,
    pub credential: ProviderCredentialBinding,
    pub routing: ProviderRoutingMetadata,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl ProviderRegistration {
    pub fn snapshot(self) -> Provider {
        let credential_ref = self
            .credential
            .handle
            .as_ref()
            .map(|handle| handle.as_str().to_string());
        Provider {
            id: self.id,
            kind: self.kind,
            label: self.label,
            status: self.status,
            selected_model: self.selected_model,
            capabilities: self.capabilities,
            credential_ref,
            credential_status: self.credential.to_status(),
            routing: self.routing,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFamily {
    OpenAi,
    Anthropic,
    Gemini,
    OpenAiCompatible,
    OpenRouter,
    LiteLlm,
    Ollama,
}

impl ProviderFamily {
    pub fn id(self) -> &'static str {
        match self {
            ProviderFamily::OpenAi => "openai",
            ProviderFamily::Anthropic => "anthropic",
            ProviderFamily::Gemini => "gemini",
            ProviderFamily::OpenAiCompatible => "openai_compatible",
            ProviderFamily::OpenRouter => "openrouter",
            ProviderFamily::LiteLlm => "litellm",
            ProviderFamily::Ollama => "ollama",
        }
    }

    fn default_label(self) -> &'static str {
        match self {
            ProviderFamily::OpenAi => "OpenAI",
            ProviderFamily::Anthropic => "Anthropic",
            ProviderFamily::Gemini => "Gemini",
            ProviderFamily::OpenAiCompatible => "OpenAI-compatible endpoint",
            ProviderFamily::OpenRouter => "OpenRouter",
            ProviderFamily::LiteLlm => "LiteLLM",
            ProviderFamily::Ollama => "Ollama local endpoint",
        }
    }

    fn kind(self) -> ProviderKind {
        match self {
            ProviderFamily::OpenAi
            | ProviderFamily::Anthropic
            | ProviderFamily::Gemini
            | ProviderFamily::OpenRouter => ProviderKind::Byok,
            ProviderFamily::OpenAiCompatible | ProviderFamily::LiteLlm => {
                ProviderKind::OpenAiCompatible
            }
            ProviderFamily::Ollama => ProviderKind::LocalModel,
        }
    }

    fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderFamily::OpenAi => Some("https://api.openai.com/v1"),
            ProviderFamily::Anthropic => Some("https://api.anthropic.com"),
            ProviderFamily::Gemini => Some("https://generativelanguage.googleapis.com"),
            ProviderFamily::OpenAiCompatible => None,
            ProviderFamily::OpenRouter => Some("https://openrouter.ai/api/v1"),
            ProviderFamily::LiteLlm => Some("http://127.0.0.1:4000/v1"),
            ProviderFamily::Ollama => Some("http://127.0.0.1:11434/v1"),
        }
    }

    fn default_capabilities(self) -> Vec<ProviderCapability> {
        match self {
            ProviderFamily::OpenAi
            | ProviderFamily::Anthropic
            | ProviderFamily::Gemini
            | ProviderFamily::OpenAiCompatible
            | ProviderFamily::OpenRouter
            | ProviderFamily::LiteLlm => vec![
                ProviderCapability::Text,
                ProviderCapability::ToolUse,
                ProviderCapability::FileInput,
                ProviderCapability::Vision,
                ProviderCapability::LongContext,
                ProviderCapability::Streaming,
            ],
            ProviderFamily::Ollama => vec![
                ProviderCapability::Text,
                ProviderCapability::ToolUse,
                ProviderCapability::Streaming,
            ],
        }
    }

    fn default_tool_support(self) -> ProviderToolSupport {
        match self {
            ProviderFamily::Ollama => ProviderToolSupport {
                tool_use: true,
                file_input: false,
                file_output: false,
                vision: false,
                streaming: true,
            },
            _ => ProviderToolSupport {
                tool_use: true,
                file_input: true,
                file_output: false,
                vision: true,
                streaming: true,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointConfig {
    pub id: String,
    pub family: ProviderFamily,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    pub selected_model: String,
    pub credential: ProviderCredentialBinding,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub capabilities: Vec<ProviderCapability>,
    #[serde(default)]
    pub routing: ProviderRoutingMetadata,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl ProviderEndpointConfig {
    pub fn byok_api_key(
        id: impl Into<String>,
        family: ProviderFamily,
        selected_model: impl Into<String>,
        handle: CredentialHandle,
        label: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            family,
            label: None,
            base_url: None,
            request_url: None,
            selected_model: selected_model.into(),
            credential: ProviderCredentialBinding::available(
                ProviderAuthMethod::ApiKey,
                handle,
                label,
            ),
            enabled: true,
            capabilities: Vec::new(),
            routing: ProviderRoutingMetadata::default(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn local_endpoint(
        id: impl Into<String>,
        family: ProviderFamily,
        selected_model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            family,
            label: None,
            base_url: None,
            request_url: None,
            selected_model: selected_model.into(),
            credential: ProviderCredentialBinding::unavailable(
                ProviderAuthMethod::LocalEndpoint,
                ProviderCredentialState::NotRequired,
            ),
            enabled: true,
            capabilities: Vec::new(),
            routing: ProviderRoutingMetadata::default(),
            metadata: BTreeMap::new(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCredentialBinding {
    pub method: ProviderAuthMethod,
    pub state: ProviderCredentialState,
    pub handle: Option<CredentialHandle>,
    pub label: Option<String>,
}

impl ProviderCredentialBinding {
    pub fn unavailable(method: ProviderAuthMethod, state: ProviderCredentialState) -> Self {
        Self {
            method,
            state,
            handle: None,
            label: None,
        }
    }

    pub fn available(
        method: ProviderAuthMethod,
        handle: CredentialHandle,
        label: impl Into<String>,
    ) -> Self {
        Self {
            method,
            state: ProviderCredentialState::Available,
            handle: Some(handle),
            label: Some(label.into()),
        }
    }

    fn to_status(&self) -> ProviderCredentialStatus {
        ProviderCredentialStatus {
            method: self.method,
            state: self.state,
            handle: self
                .handle
                .as_ref()
                .map(|handle| handle.as_str().to_string()),
            label: self.label.clone(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderRegistryError {
    #[error("provider id is invalid")]
    InvalidProviderId,
    #[error("provider is already registered: {provider_id}")]
    DuplicateProvider { provider_id: String },
    #[error("provider was not found: {provider_id}")]
    ProviderNotFound { provider_id: String },
    #[error("provider endpoint is invalid for {provider_id}: {endpoint}")]
    InvalidProviderEndpoint {
        provider_id: String,
        endpoint: String,
    },
    #[error("provider credential is missing for {provider_id}")]
    MissingProviderCredential { provider_id: String },
    #[error("provider is unavailable for selection: {provider_id} ({status:?})")]
    ProviderUnavailable {
        provider_id: String,
        status: ProviderStatus,
    },
    #[error("provider capability mismatch for {provider_id}: missing {missing:?}")]
    CapabilityMismatch {
        provider_id: String,
        missing: Vec<ProviderCapability>,
    },
    #[error("provider requirement mismatch for {provider_id}")]
    RequirementMismatch {
        provider_id: String,
        mismatch: ProviderCapabilityMismatch,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilityRequirement {
    pub capabilities: Vec<ProviderCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_max_output_tokens: Option<u32>,
    pub tool_support: ProviderToolSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilityMismatch {
    pub missing_capabilities: Vec<ProviderCapability>,
    pub missing_tool_support: Vec<ProviderToolFeature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_max_output_tokens: Option<u32>,
}

impl ProviderCapabilityMismatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolFeature {
    ToolUse,
    FileInput,
    FileOutput,
    Vision,
    Streaming,
}

impl ProviderRegistry {
    pub fn register(
        &mut self,
        registration: ProviderRegistration,
    ) -> Result<(), ProviderRegistryError> {
        validate_provider_id(&registration.id)?;
        let provider = registration.snapshot();
        if self.providers.contains_key(&provider.id) {
            return Err(ProviderRegistryError::DuplicateProvider {
                provider_id: provider.id,
            });
        }
        self.providers.insert(provider.id.clone(), provider);
        Ok(())
    }

    pub fn register_endpoint_config(
        &mut self,
        config: ProviderEndpointConfig,
    ) -> Result<(), ProviderRegistryError> {
        let registration = endpoint_config_to_registration(config)?;
        self.register(registration)
    }

    pub fn provider(&self, provider_id: &str) -> Option<&Provider> {
        self.providers.get(provider_id)
    }

    pub fn select_provider(&self, provider_id: &str) -> Result<&Provider, ProviderRegistryError> {
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            ProviderRegistryError::ProviderNotFound {
                provider_id: provider_id.to_string(),
            }
        })?;
        match provider.status {
            ProviderStatus::Available => {}
            status => {
                return Err(ProviderRegistryError::ProviderUnavailable {
                    provider_id: provider_id.to_string(),
                    status,
                });
            }
        }
        if provider.credential_status.state == ProviderCredentialState::Missing
            || (provider.credential_status.method == ProviderAuthMethod::ApiKey
                && provider.credential_status.handle.is_none())
        {
            return Err(ProviderRegistryError::MissingProviderCredential {
                provider_id: provider_id.to_string(),
            });
        }
        Ok(provider)
    }

    pub fn snapshot(&self) -> Vec<Provider> {
        self.providers.values().cloned().collect()
    }

    pub fn ensure_capabilities(
        &self,
        provider_id: &str,
        required: &[ProviderCapability],
    ) -> Result<(), ProviderRegistryError> {
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            ProviderRegistryError::ProviderNotFound {
                provider_id: provider_id.to_string(),
            }
        })?;
        let missing = required
            .iter()
            .copied()
            .filter(|required| !provider.capabilities.contains(required))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ProviderRegistryError::CapabilityMismatch {
                provider_id: provider_id.to_string(),
                missing,
            })
        }
    }

    pub fn ensure_requirement(
        &self,
        provider_id: &str,
        requirement: &ProviderCapabilityRequirement,
    ) -> Result<(), ProviderRegistryError> {
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            ProviderRegistryError::ProviderNotFound {
                provider_id: provider_id.to_string(),
            }
        })?;
        let mismatch = requirement_mismatch(provider, requirement);
        if mismatch.is_empty() {
            Ok(())
        } else {
            Err(ProviderRegistryError::RequirementMismatch {
                provider_id: provider_id.to_string(),
                mismatch,
            })
        }
    }
}

pub fn supported_provider_families() -> Vec<ProviderFamily> {
    vec![
        ProviderFamily::OpenAi,
        ProviderFamily::Anthropic,
        ProviderFamily::Gemini,
        ProviderFamily::OpenAiCompatible,
        ProviderFamily::OpenRouter,
        ProviderFamily::LiteLlm,
        ProviderFamily::Ollama,
    ]
}

fn endpoint_config_to_registration(
    config: ProviderEndpointConfig,
) -> Result<ProviderRegistration, ProviderRegistryError> {
    validate_provider_id(&config.id)?;
    if config.selected_model.trim().is_empty() {
        return Err(ProviderRegistryError::InvalidProviderEndpoint {
            provider_id: config.id,
            endpoint: "missing selected model".to_string(),
        });
    }

    let base_url = config
        .base_url
        .clone()
        .or_else(|| config.family.default_base_url().map(str::to_string))
        .ok_or_else(|| ProviderRegistryError::InvalidProviderEndpoint {
            provider_id: config.id.clone(),
            endpoint: "missing base URL".to_string(),
        })?;
    validate_endpoint(&config.id, &base_url)?;

    let request_url = config
        .request_url
        .clone()
        .unwrap_or_else(|| default_request_url(config.family, &base_url, &config.selected_model));
    validate_endpoint(&config.id, &request_url)?;

    let requires_api_key = config.credential.method == ProviderAuthMethod::ApiKey;
    if requires_api_key
        && (config.credential.state != ProviderCredentialState::Available
            || config.credential.handle.is_none())
    {
        return Err(ProviderRegistryError::MissingProviderCredential {
            provider_id: config.id,
        });
    }

    let mut metadata = config.metadata;
    metadata.insert(
        PROVIDER_FAMILY_METADATA_KEY.to_string(),
        serde_json::Value::String(config.family.id().to_string()),
    );
    metadata.insert(
        BASE_URL_METADATA_KEY.to_string(),
        serde_json::Value::String(base_url),
    );
    metadata.insert(
        REQUEST_URL_METADATA_KEY.to_string(),
        serde_json::Value::String(request_url),
    );

    let mut routing = config.routing;
    if routing.tool_support.is_empty() {
        routing.tool_support = config.family.default_tool_support();
    }

    Ok(ProviderRegistration {
        id: config.id,
        kind: config.family.kind(),
        label: config
            .label
            .unwrap_or_else(|| config.family.default_label().to_string()),
        status: if config.enabled {
            ProviderStatus::Available
        } else {
            ProviderStatus::Disabled
        },
        selected_model: Some(config.selected_model),
        capabilities: if config.capabilities.is_empty() {
            config.family.default_capabilities()
        } else {
            config.capabilities
        },
        credential: config.credential,
        routing,
        metadata,
    })
}

fn default_request_url(family: ProviderFamily, base_url: &str, selected_model: &str) -> String {
    match family {
        ProviderFamily::OpenAi
        | ProviderFamily::OpenAiCompatible
        | ProviderFamily::OpenRouter
        | ProviderFamily::LiteLlm
        | ProviderFamily::Ollama => append_endpoint(base_url, "chat/completions"),
        ProviderFamily::Anthropic => append_endpoint(base_url, "v1/messages"),
        ProviderFamily::Gemini => append_endpoint(
            base_url,
            &format!(
                "v1beta/models/{}:streamGenerateContent?alt=sse",
                selected_model.trim()
            ),
        ),
    }
}

fn append_endpoint(base_url: &str, endpoint: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with(endpoint) {
        trimmed.to_string()
    } else {
        format!("{trimmed}/{}", endpoint.trim_start_matches('/'))
    }
}

fn validate_endpoint(provider_id: &str, endpoint: &str) -> Result<(), ProviderRegistryError> {
    let trimmed = endpoint.trim();
    let invalid = trimmed.is_empty()
        || trimmed.chars().any(char::is_whitespace)
        || trimmed.chars().any(char::is_control)
        || !(trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        || trimmed
            .split_once("://")
            .and_then(|(_, rest)| rest.split('/').next())
            .map(str::is_empty)
            .unwrap_or(true);
    if invalid {
        Err(ProviderRegistryError::InvalidProviderEndpoint {
            provider_id: provider_id.to_string(),
            endpoint: REDACTED.to_string(),
        })
    } else {
        Ok(())
    }
}

fn requirement_mismatch(
    provider: &Provider,
    requirement: &ProviderCapabilityRequirement,
) -> ProviderCapabilityMismatch {
    let missing_capabilities = requirement
        .capabilities
        .iter()
        .copied()
        .filter(|required| !provider.capabilities.contains(required))
        .collect::<Vec<_>>();

    let mut missing_tool_support = Vec::new();
    collect_missing_tool_support(
        requirement.tool_support.tool_use,
        provider.routing.tool_support.tool_use,
        ProviderToolFeature::ToolUse,
        &mut missing_tool_support,
    );
    collect_missing_tool_support(
        requirement.tool_support.file_input,
        provider.routing.tool_support.file_input,
        ProviderToolFeature::FileInput,
        &mut missing_tool_support,
    );
    collect_missing_tool_support(
        requirement.tool_support.file_output,
        provider.routing.tool_support.file_output,
        ProviderToolFeature::FileOutput,
        &mut missing_tool_support,
    );
    collect_missing_tool_support(
        requirement.tool_support.vision,
        provider.routing.tool_support.vision,
        ProviderToolFeature::Vision,
        &mut missing_tool_support,
    );
    collect_missing_tool_support(
        requirement.tool_support.streaming,
        provider.routing.tool_support.streaming,
        ProviderToolFeature::Streaming,
        &mut missing_tool_support,
    );

    let context_mismatch = requirement
        .min_context_window_tokens
        .zip(provider.routing.context_window_tokens)
        .filter(|(required, actual)| actual < required);
    let missing_context = requirement
        .min_context_window_tokens
        .filter(|_| provider.routing.context_window_tokens.is_none());
    let max_output_mismatch = requirement
        .min_max_output_tokens
        .zip(provider.routing.max_output_tokens)
        .filter(|(required, actual)| actual < required);
    let missing_max_output = requirement
        .min_max_output_tokens
        .filter(|_| provider.routing.max_output_tokens.is_none());

    ProviderCapabilityMismatch {
        missing_capabilities,
        missing_tool_support,
        required_context_window_tokens: context_mismatch
            .map(|(required, _)| required)
            .or(missing_context),
        actual_context_window_tokens: context_mismatch.map(|(_, actual)| actual).or(provider
            .routing
            .context_window_tokens
            .filter(|_| missing_context.is_some())),
        required_max_output_tokens: max_output_mismatch
            .map(|(required, _)| required)
            .or(missing_max_output),
        actual_max_output_tokens: max_output_mismatch.map(|(_, actual)| actual).or(provider
            .routing
            .max_output_tokens
            .filter(|_| missing_max_output.is_some())),
    }
}

fn collect_missing_tool_support(
    required: bool,
    actual: bool,
    feature: ProviderToolFeature,
    missing: &mut Vec<ProviderToolFeature>,
) {
    if required && !actual {
        missing.push(feature);
    }
}

pub fn routing_metadata(
    context_window_tokens: u32,
    max_output_tokens: u32,
    tool_support: ProviderToolSupport,
    rate_limits: ProviderRateLimits,
    cost: ProviderCost,
) -> ProviderRoutingMetadata {
    ProviderRoutingMetadata {
        context_window_tokens: Some(context_window_tokens),
        max_output_tokens: Some(max_output_tokens),
        tool_support,
        rate_limits,
        cost,
    }
}

fn validate_token(value: &str) -> Result<(), CredentialVaultError> {
    if value.trim().is_empty()
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        Err(CredentialVaultError::InvalidHandle)
    } else {
        Ok(())
    }
}

fn validate_provider_id(provider_id: &str) -> Result<(), ProviderRegistryError> {
    if provider_id.trim().is_empty()
        || provider_id.chars().any(char::is_whitespace)
        || provider_id.chars().any(char::is_control)
    {
        Err(ProviderRegistryError::InvalidProviderId)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_secret() -> &'static str {
        "synthetic-secret-cowork-provider-registry-1234567890"
    }

    fn registration(handle: CredentialHandle) -> ProviderRegistration {
        ProviderRegistration {
            id: "openai-compatible".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            label: "OpenAI-compatible gateway".to_string(),
            status: ProviderStatus::Available,
            selected_model: Some("cowork-model".to_string()),
            capabilities: vec![
                ProviderCapability::Text,
                ProviderCapability::ToolUse,
                ProviderCapability::FileInput,
                ProviderCapability::Streaming,
            ],
            credential: ProviderCredentialBinding::available(
                ProviderAuthMethod::ApiKey,
                handle,
                "gateway-key",
            ),
            routing: routing_metadata(
                128_000,
                16_384,
                ProviderToolSupport {
                    tool_use: true,
                    file_input: true,
                    file_output: false,
                    vision: false,
                    streaming: true,
                },
                ProviderRateLimits {
                    requests_per_minute: Some(120),
                    tokens_per_minute: Some(1_000_000),
                },
                ProviderCost {
                    currency: Some("USD".to_string()),
                    input_per_million_micros: Some(250_000),
                    output_per_million_micros: Some(1_000_000),
                    cached_input_per_million_micros: Some(25_000),
                },
            ),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn vault_stores_and_retrieves_secret_only_by_handle() {
        let handle = CredentialHandle::for_provider("openai-compatible", "api-key")
            .expect("synthetic handle");
        let mut vault = LocalCredentialVault::memory_only();
        vault.store(
            handle.clone(),
            CredentialSecret::new(synthetic_secret()).expect("synthetic secret"),
        );

        let retrieved = vault.retrieve(&handle).expect("secret by handle");
        assert_eq!(retrieved.expose_secret(), synthetic_secret());
        assert!(!format!("{retrieved:?}").contains(synthetic_secret()));
    }

    #[test]
    fn unsupported_backend_reports_typed_unavailable_state() {
        let error = LocalCredentialVault::new(vec![
            VaultBackendStatus::blocked(VaultBackend::Keychain, "not available in CI"),
            VaultBackendStatus::blocked(VaultBackend::EncryptedFile, "not configured"),
        ])
        .expect_err("no backend should be available");

        match error {
            CredentialVaultError::BackendUnavailable { backends } => {
                assert_eq!(backends.len(), 2);
                assert!(backends.iter().all(|backend| !backend.available));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn invalid_credential_secret_returns_typed_error_without_echoing_secret() {
        let error =
            CredentialSecret::new(" \n\t ").expect_err("blank credential material is invalid");
        assert_eq!(error, CredentialVaultError::InvalidSecret);
        assert!(!format!("{error:?}").contains("\\n"));
    }

    #[test]
    fn registry_snapshot_includes_routing_metadata_without_raw_secret() {
        let handle = CredentialHandle::for_provider("openai-compatible", "api-key")
            .expect("synthetic handle");
        let mut vault = LocalCredentialVault::memory_only();
        vault.store(
            handle.clone(),
            CredentialSecret::new(synthetic_secret()).expect("synthetic secret"),
        );

        let mut registry = ProviderRegistry::default();
        registry
            .register(registration(handle))
            .expect("provider registration");
        let snapshot = registry.snapshot();
        let json = serde_json::to_string_pretty(&snapshot).expect("snapshot json");

        assert!(json.contains("\"contextWindowTokens\": 128000"));
        assert!(json.contains("\"toolUse\": true"));
        assert!(json.contains("\"streaming\": true"));
        assert!(json.contains("\"inputPerMillionMicros\": 250000"));
        assert!(json.contains("cowork-vault://providers/openai-compatible/api-key"));
        assert!(!json.contains(synthetic_secret()));
    }

    #[test]
    fn capability_mismatch_returns_typed_blocked_error() {
        let handle = CredentialHandle::for_provider("openai-compatible", "api-key")
            .expect("synthetic handle");
        let mut registry = ProviderRegistry::default();
        registry
            .register(registration(handle))
            .expect("provider registration");

        let error = registry
            .ensure_capabilities("openai-compatible", &[ProviderCapability::Vision])
            .expect_err("vision should be missing");

        assert!(matches!(
            error,
            ProviderRegistryError::CapabilityMismatch {
                provider_id,
                missing
            } if provider_id == "openai-compatible" && missing == vec![ProviderCapability::Vision]
        ));
    }

    #[test]
    fn multidimensional_requirement_reports_precise_mismatch() {
        let handle = CredentialHandle::for_provider("openai-compatible", "api-key")
            .expect("synthetic handle");
        let mut registry = ProviderRegistry::default();
        registry
            .register(registration(handle))
            .expect("provider registration");

        let requirement = ProviderCapabilityRequirement {
            capabilities: vec![ProviderCapability::Text, ProviderCapability::Vision],
            min_context_window_tokens: Some(200_000),
            min_max_output_tokens: Some(32_000),
            tool_support: ProviderToolSupport {
                tool_use: true,
                file_input: true,
                file_output: true,
                vision: true,
                streaming: true,
            },
        };

        let error = registry
            .ensure_requirement("openai-compatible", &requirement)
            .expect_err("vision, context, output, and file-output support should be missing");

        match error {
            ProviderRegistryError::RequirementMismatch {
                provider_id,
                mismatch,
            } => {
                assert_eq!(provider_id, "openai-compatible");
                assert_eq!(
                    mismatch.missing_capabilities,
                    vec![ProviderCapability::Vision]
                );
                assert_eq!(
                    mismatch.missing_tool_support,
                    vec![ProviderToolFeature::FileOutput, ProviderToolFeature::Vision]
                );
                assert_eq!(mismatch.required_context_window_tokens, Some(200_000));
                assert_eq!(mismatch.actual_context_window_tokens, Some(128_000));
                assert_eq!(mismatch.required_max_output_tokens, Some(32_000));
                assert_eq!(mismatch.actual_max_output_tokens, Some(16_384));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn supported_provider_catalog_covers_required_families() {
        assert_eq!(
            supported_provider_families(),
            vec![
                ProviderFamily::OpenAi,
                ProviderFamily::Anthropic,
                ProviderFamily::Gemini,
                ProviderFamily::OpenAiCompatible,
                ProviderFamily::OpenRouter,
                ProviderFamily::LiteLlm,
                ProviderFamily::Ollama,
            ]
        );
    }

    #[test]
    fn every_supported_family_registers_as_configured_provider() {
        let mut registry = ProviderRegistry::default();
        for family in supported_provider_families() {
            let provider_id = format!("{}-provider", family.id());
            let mut config = if family == ProviderFamily::Ollama {
                ProviderEndpointConfig::local_endpoint(&provider_id, family, "local-model")
            } else {
                ProviderEndpointConfig::byok_api_key(
                    &provider_id,
                    family,
                    "provider-model",
                    CredentialHandle::for_provider(&provider_id, "api-key")
                        .expect("provider handle"),
                    "provider-key",
                )
            };
            if family == ProviderFamily::OpenAiCompatible {
                config.base_url = Some("https://gateway.example.test/v1".to_string());
            }
            registry
                .register_endpoint_config(config)
                .expect("family should register as configured provider");
        }

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), supported_provider_families().len());
        for family in supported_provider_families() {
            let provider_id = format!("{}-provider", family.id());
            let provider = registry
                .select_provider(&provider_id)
                .expect("registered provider should be selectable");
            assert_eq!(
                provider.metadata.get(PROVIDER_FAMILY_METADATA_KEY),
                Some(&serde_json::Value::String(family.id().to_string()))
            );
        }
    }

    #[test]
    fn daemon_selection_uses_configured_provider_without_secret_leakage() {
        let handle =
            CredentialHandle::for_provider("openrouter-main", "api-key").expect("synthetic handle");
        let mut vault = LocalCredentialVault::memory_only();
        vault.store(
            handle.clone(),
            CredentialSecret::new(synthetic_secret()).expect("synthetic secret"),
        );

        let mut registry = ProviderRegistry::default();
        registry
            .register_endpoint_config(ProviderEndpointConfig::byok_api_key(
                "openrouter-main",
                ProviderFamily::OpenRouter,
                "openrouter/auto",
                handle,
                "openrouter-key",
            ))
            .expect("provider config should register");

        let provider = registry
            .select_provider("openrouter-main")
            .expect("daemon can select provider by id");
        assert_eq!(provider.kind, ProviderKind::Byok);
        assert_eq!(provider.status, ProviderStatus::Available);
        assert_eq!(
            provider.metadata.get(PROVIDER_FAMILY_METADATA_KEY),
            Some(&serde_json::Value::String("openrouter".to_string()))
        );
        assert_eq!(
            provider.metadata.get(REQUEST_URL_METADATA_KEY),
            Some(&serde_json::Value::String(
                "https://openrouter.ai/api/v1/chat/completions".to_string()
            ))
        );

        let evidence_text = serde_json::to_string_pretty(provider).expect("provider serializes");
        assert!(evidence_text.contains("cowork-vault://providers/openrouter-main/api-key"));
        assert!(!evidence_text.contains(synthetic_secret()));
    }

    #[test]
    fn invalid_endpoint_returns_typed_error_with_redacted_endpoint() {
        let handle =
            CredentialHandle::for_provider("bad-provider", "api-key").expect("synthetic handle");
        let mut config = ProviderEndpointConfig::byok_api_key(
            "bad-provider",
            ProviderFamily::OpenAiCompatible,
            "model",
            handle,
            "bad-key",
        );
        config.base_url =
            Some("not a url with synthetic-secret-cowork-provider-registry-1234567890".into());

        let mut registry = ProviderRegistry::default();
        let error = registry
            .register_endpoint_config(config)
            .expect_err("invalid endpoint should be rejected");

        assert!(matches!(
            error,
            ProviderRegistryError::InvalidProviderEndpoint {
                ref provider_id,
                ref endpoint
            } if provider_id == "bad-provider" && endpoint == REDACTED
        ));
        assert!(!format!("{error:?}").contains(synthetic_secret()));
    }

    #[test]
    fn missing_byok_credential_blocks_selection_as_typed_error() {
        let mut registry = ProviderRegistry::default();
        let error = registry
            .register_endpoint_config(ProviderEndpointConfig {
                id: "openai-missing".to_string(),
                family: ProviderFamily::OpenAi,
                label: None,
                base_url: None,
                request_url: None,
                selected_model: "gpt-4.1".to_string(),
                credential: ProviderCredentialBinding::unavailable(
                    ProviderAuthMethod::ApiKey,
                    ProviderCredentialState::Missing,
                ),
                enabled: true,
                capabilities: Vec::new(),
                routing: ProviderRoutingMetadata::default(),
                metadata: BTreeMap::new(),
            })
            .expect_err("missing BYOK credential should be rejected");

        assert!(matches!(
            error,
            ProviderRegistryError::MissingProviderCredential { provider_id }
                if provider_id == "openai-missing"
        ));
    }

    #[test]
    fn ollama_local_endpoint_can_be_selected_without_api_key() {
        let mut registry = ProviderRegistry::default();
        registry
            .register_endpoint_config(ProviderEndpointConfig::local_endpoint(
                "ollama-local",
                ProviderFamily::Ollama,
                "llama3.2",
            ))
            .expect("ollama local provider should not require api key");

        let provider = registry
            .select_provider("ollama-local")
            .expect("daemon can select local endpoint");
        assert_eq!(provider.kind, ProviderKind::LocalModel);
        assert_eq!(
            provider.credential_status.method,
            ProviderAuthMethod::LocalEndpoint
        );
        assert_eq!(
            provider.credential_status.state,
            ProviderCredentialState::NotRequired
        );
    }
}
