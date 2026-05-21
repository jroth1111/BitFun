//! Local provider credential handles and capability registry for Cowork.
//!
//! This crate deliberately separates secret material from provider snapshots.
//! Prompt-visible exports receive opaque handles, credential state, and routing
//! metadata; raw provider keys only pass through vault store/retrieve APIs.

use cowork_protocol::{
    Provider, ProviderAuthMethod, ProviderCapability, ProviderCost, ProviderCredentialState,
    ProviderCredentialStatus, ProviderKind, ProviderRateLimits, ProviderRoutingMetadata,
    ProviderStatus, ProviderToolSupport,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

const REDACTED: &str = "[REDACTED]";

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
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    #[error("provider capability mismatch for {provider_id}: missing {missing:?}")]
    CapabilityMismatch {
        provider_id: String,
        missing: Vec<ProviderCapability>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Provider>,
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

    pub fn provider(&self, provider_id: &str) -> Option<&Provider> {
        self.providers.get(provider_id)
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
        "sk-test-cowork-provider-registry-1234567890"
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
}
