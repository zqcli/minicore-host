use std::{fmt, net::IpAddr, time::Duration};

use minicore_runtime::model::ModelDescriptor;
use reqwest::{Url, header::HeaderValue};
use thiserror::Error;

/// Host-private connection settings. Deliberately not serializable and not an
/// Agent/Session configuration. The endpoint is exact, not a guessed base URL.
#[derive(Clone)]
pub struct OpenAiChatConfig {
    pub endpoint: String,
    pub provider_model: String,
    pub descriptor: ModelDescriptor,
    /// Optional transport timeout, subordinate to Runtime's absolute deadline.
    /// None adds no independent total timeout or output quota.
    pub request_timeout: Option<Duration>,
}

impl fmt::Debug for OpenAiChatConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiChatConfig")
            .field("descriptor", &self.descriptor)
            .field("request_timeout", &self.request_timeout)
            .finish_non_exhaustive()
    }
}

impl OpenAiChatConfig {
    pub fn new(
        endpoint: impl Into<String>,
        provider_model: impl Into<String>,
        descriptor: ModelDescriptor,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            provider_model: provider_model.into(),
            descriptor,
            request_timeout: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<Url, ProviderConfigError> {
        self.descriptor
            .validate()
            .map_err(|_| ProviderConfigError::InvalidDescriptor)?;
        if self.provider_model.trim().is_empty()
            || self.provider_model.chars().any(char::is_control)
            || self
                .request_timeout
                .is_some_and(|timeout| timeout.is_zero())
        {
            return Err(ProviderConfigError::InvalidConfiguration);
        }
        let endpoint =
            Url::parse(&self.endpoint).map_err(|_| ProviderConfigError::InvalidEndpoint)?;
        let loopback = endpoint.host_str().is_some_and(|host| {
            // Literal loopback addresses only: DNS cannot turn a plaintext
            // test exception into a remote credential transmission.
            host.trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if endpoint.host().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback))
        {
            return Err(ProviderConfigError::InvalidEndpoint);
        }
        Ok(endpoint)
    }
}

/// A credential that cannot be serialized or exposed by Debug. It is consumed
/// into a sensitive HTTP header; it never belongs in resource descriptors.
pub struct SecretApiKey(String);

impl SecretApiKey {
    pub fn new(value: impl Into<String>) -> Result<Self, ProviderConfigError> {
        let value = value.into();
        if value.is_empty() || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err(ProviderConfigError::InvalidCredential);
        }
        Ok(Self(value))
    }

    pub fn from_env(name: &str) -> Result<Self, ProviderConfigError> {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(ProviderConfigError::MissingCredential);
        }
        let value = std::env::var(name).map_err(|_| ProviderConfigError::MissingCredential)?;
        Self::new(value)
    }

    pub(crate) fn into_authorization(self) -> Result<HeaderValue, ProviderConfigError> {
        let mut header = HeaderValue::from_str(&format!("Bearer {}", self.0))
            .map_err(|_| ProviderConfigError::InvalidCredential)?;
        header.set_sensitive(true);
        Ok(header)
    }
}

impl fmt::Debug for SecretApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretApiKey([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProviderConfigError {
    #[error("model descriptor is invalid")]
    InvalidDescriptor,
    #[error("provider configuration is invalid")]
    InvalidConfiguration,
    #[error(
        "endpoint must be HTTPS (or literal loopback HTTP), without userinfo, query or fragment"
    )]
    InvalidEndpoint,
    #[error("provider credential is missing")]
    MissingCredential,
    #[error("provider credential is invalid")]
    InvalidCredential,
    #[error("provider HTTP client initialization failed")]
    ClientBuild,
}

#[cfg(test)]
mod tests {
    use super::*;
    use minicore_runtime::model::ReasoningPreference;
    use std::collections::BTreeSet;

    fn config(endpoint: &str) -> OpenAiChatConfig {
        OpenAiChatConfig::new(
            endpoint,
            "test",
            ModelDescriptor::new(
                "fixture/chat".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
        )
    }

    #[test]
    fn only_secure_or_literal_loopback_endpoints_are_accepted() {
        for endpoint in [
            "https://example.com/v1/chat/completions",
            "http://127.0.0.1:8000/chat/completions",
            "http://[::1]/chat/completions",
        ] {
            assert!(config(endpoint).validate().is_ok());
        }
        for endpoint in [
            "http://example.com/",
            "http://localhost/",
            "https://key@example.com/",
            "https://example.com/?api_key=secret",
            "https://example.com/#secret",
            "file:///tmp/provider",
        ] {
            assert_eq!(
                config(endpoint).validate().unwrap_err(),
                ProviderConfigError::InvalidEndpoint
            );
        }
    }

    #[test]
    fn credentials_and_unvalidated_endpoints_are_redacted() {
        let key = SecretApiKey::new("super-secret-key").unwrap();
        assert!(!format!("{key:?}").contains("super-secret-key"));
        assert!(
            !format!("{:?}", config("https://example.com/?key=super-secret-key"))
                .contains("super-secret-key")
        );
        let header = key.into_authorization().unwrap();
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains("super-secret-key"));
        for key in ["", "not a key", "bad\r\nheader"] {
            assert!(SecretApiKey::new(key).is_err());
        }
    }
}
