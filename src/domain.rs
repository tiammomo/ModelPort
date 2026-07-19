use std::fmt;

use uuid::Uuid;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn from_string(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(RequestId);
string_id!(AttemptId);
string_id!(OrganizationId);
string_id!(ProjectId);
string_id!(EnvironmentId);
string_id!(PrincipalId);

pub(crate) fn valid_tenant_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

impl RequestId {
    pub(crate) fn new() -> Self {
        Self::from_string(format!("req_{}", Uuid::new_v4().simple()))
    }

    pub(crate) fn from_external_or_new(value: Option<&str>) -> Self {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(Self::from_string)
            .unwrap_or_else(Self::new)
    }
}

impl AttemptId {
    pub(crate) fn new() -> Self {
        Self::from_string(format!("att_{}", Uuid::new_v4().simple()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TenantScope {
    pub(crate) organization_id: OrganizationId,
    pub(crate) project_id: ProjectId,
    pub(crate) environment_id: EnvironmentId,
}

impl TenantScope {
    pub(crate) fn legacy_local() -> Self {
        Self {
            organization_id: OrganizationId::from_string("org_local"),
            project_id: ProjectId::from_string("prj_default"),
            environment_id: EnvironmentId::from_string("env_default"),
        }
    }

    pub(crate) fn from_strings(
        organization_id: impl Into<String>,
        project_id: impl Into<String>,
        environment_id: impl Into<String>,
    ) -> Self {
        Self {
            organization_id: OrganizationId::from_string(organization_id),
            project_id: ProjectId::from_string(project_id),
            environment_id: EnvironmentId::from_string(environment_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientProtocol {
    AnthropicMessages,
    OpenAiChatCompletions,
}

impl ClientProtocol {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic-messages",
            Self::OpenAiChatCompletions => "openai-chat-completions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestContext {
    pub(crate) request_id: RequestId,
    pub(crate) tenant: TenantScope,
    pub(crate) principal_id: PrincipalId,
    pub(crate) protocol: ClientProtocol,
}

impl RequestContext {
    pub(crate) fn scoped(
        request_id: RequestId,
        tenant: TenantScope,
        principal_id: impl Into<String>,
        protocol: ClientProtocol,
    ) -> Self {
        Self {
            request_id,
            tenant,
            principal_id: PrincipalId::from_string(principal_id),
            protocol,
        }
    }

    #[cfg(test)]
    pub(crate) fn legacy(
        request_id: RequestId,
        principal_id: impl Into<String>,
        protocol: ClientProtocol,
    ) -> Self {
        Self::scoped(
            request_id,
            TenantScope::legacy_local(),
            principal_id,
            protocol,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_request_id_is_preserved_when_bounded() {
        let request_id = RequestId::from_external_or_new(Some(" client-request-1 "));
        assert_eq!(request_id.as_str(), "client-request-1");
    }

    #[test]
    fn missing_or_unbounded_request_id_is_replaced() {
        assert!(
            RequestId::from_external_or_new(None)
                .as_str()
                .starts_with("req_")
        );
        assert!(
            RequestId::from_external_or_new(Some(&"x".repeat(257)))
                .as_str()
                .starts_with("req_")
        );
    }

    #[test]
    fn legacy_context_has_explicit_tenant_scope() {
        let context = RequestContext::legacy(
            RequestId::from_string("req_test"),
            "usr_local_admin",
            ClientProtocol::AnthropicMessages,
        );
        assert_eq!(context.tenant.organization_id.as_str(), "org_local");
        assert_eq!(context.tenant.project_id.as_str(), "prj_default");
        assert_eq!(context.tenant.environment_id.as_str(), "env_default");
        assert_eq!(context.protocol.as_str(), "anthropic-messages");
    }

    #[test]
    fn tenant_identifiers_are_bounded_ascii_scope_values() {
        assert!(valid_tenant_identifier("org.example:prod-1"));
        assert!(!valid_tenant_identifier(""));
        assert!(!valid_tenant_identifier("org with spaces"));
        assert!(!valid_tenant_identifier("组织"));
        assert!(!valid_tenant_identifier(&"x".repeat(129)));
    }
}
