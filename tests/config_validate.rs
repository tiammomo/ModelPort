use std::process::{Command, Output};

const DATABASE_URL: &str = "postgres://modelport:test-secret@db.example:5432/modelport";

fn run_config_validate(extra_env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_model-port"));
    command
        .args(["config", "validate"])
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .env(
            "MODELPORT_CONFIG",
            concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml"),
        )
        .env(
            "MODELPORT_ENV_FILE",
            concat!(env!("CARGO_MANIFEST_DIR"), "/target/no-test-env-file"),
        )
        .env("MODELPORT_AUTH_TOKEN", "ci-router-token-for-validation")
        .env(
            "DEEPSEEK_ANTHROPIC_AUTH_TOKEN",
            "ci-provider-token-for-validation",
        );

    for (name, value) in extra_env {
        command.env(name, value);
    }

    command.output().expect("run model-port config validate")
}

fn run_env_default_config_validate(extra_env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_model-port"));
    command
        .args(["config", "validate"])
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .env(
            "MODELPORT_CONFIG",
            concat!(env!("CARGO_MANIFEST_DIR"), "/target/no-test-config.toml"),
        )
        .env(
            "MODELPORT_ENV_FILE",
            concat!(env!("CARGO_MANIFEST_DIR"), "/target/no-test-env-file"),
        )
        .env("MODELPORT_AUTH_TOKEN", "ci-router-token-for-validation")
        .env(
            "DEEPSEEK_ANTHROPIC_AUTH_TOKEN",
            "ci-provider-token-for-validation",
        );

    for (name, value) in extra_env {
        command.env(name, value);
    }

    command.output().expect("run model-port config validate")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn cli_deployment_preflight_rejects_unsafe_enterprise_environments() {
    let missing_database = run_config_validate(&[("MODELPORT_ENTERPRISE_MODE", "1")]);
    let missing_database_text = output_text(&missing_database);
    assert!(
        !missing_database.status.success(),
        "{missing_database_text}"
    );
    assert!(missing_database_text.contains("MODELPORT_DATABASE_URL"));

    let weak_tls = run_config_validate(&[
        ("MODELPORT_ENTERPRISE_MODE", "1"),
        ("MODELPORT_DATABASE_URL", DATABASE_URL),
        ("MODELPORT_DATABASE_TLS_MODE", "disable"),
    ]);
    let weak_tls_text = output_text(&weak_tls);
    assert!(!weak_tls.status.success(), "{weak_tls_text}");
    assert!(weak_tls_text.contains("verify-full"));
    assert!(!weak_tls_text.contains("test-secret"));

    let invalid_proxy = run_config_validate(&[("MODELPORT_TRUSTED_PROXIES", "not-a-network")]);
    let invalid_proxy_text = output_text(&invalid_proxy);
    assert!(!invalid_proxy.status.success(), "{invalid_proxy_text}");
    assert!(invalid_proxy_text.contains("MODELPORT_TRUSTED_PROXIES"));
}

#[test]
fn cli_deployment_preflight_accepts_a_valid_local_environment() {
    let output = run_config_validate(&[]);
    let text = output_text(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("ModelPort configuration valid"));
}

#[test]
fn openai_legacy_server_env_names_remain_compatible_with_a_migration_warning() {
    let output = run_env_default_config_validate(&[
        ("OPENAI_BASE_URL", "https://api.openai.com/v1"),
        ("OPENAI_API_KEY", "ci-openai-provider-token"),
        ("OPENAI_MODEL", "gpt-test"),
        ("OPENAI_MODELS", "gpt-test,gpt-test-mini"),
    ]);
    let text = output_text(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("legacy client-style environment fallback"));
    assert!(text.contains("`OPENAI_API_KEY` -> `MODELPORT_OPENAI_API_KEY`"));
    assert!(text.contains("`OPENAI_BASE_URL` -> `MODELPORT_OPENAI_BASE_URL`"));
}

#[test]
fn modelport_openai_env_names_take_precedence_over_legacy_client_names() {
    let output = run_env_default_config_validate(&[
        ("MODELPORT_OPENAI_BASE_URL", "https://api.openai.com/v1"),
        ("MODELPORT_OPENAI_API_KEY", "ci-openai-provider-token"),
        ("MODELPORT_OPENAI_MODEL", "gpt-primary"),
        ("MODELPORT_OPENAI_MODELS", "gpt-primary"),
        ("OPENAI_BASE_URL", "http://127.0.0.1:17878/v1"),
        ("OPENAI_API_KEY", "legacy-client-token"),
        ("OPENAI_MODEL", "legacy-client-model"),
        ("OPENAI_MODELS", "legacy-client-model"),
    ]);
    let text = output_text(&output);

    assert!(output.status.success(), "{text}");
    assert!(!text.contains("legacy client-style environment fallback"));
    assert!(!text.contains("points back to this ModelPort listener"));
}

#[test]
fn openai_upstream_base_url_cannot_point_back_to_modelport() {
    let output = run_env_default_config_validate(&[
        ("MODELPORT_OPENAI_BASE_URL", "http://127.0.0.1:17878/v1"),
        ("MODELPORT_OPENAI_API_KEY", "ci-openai-provider-token"),
        ("MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS", "1"),
    ]);
    let text = output_text(&output);

    assert!(!output.status.success(), "{text}");
    assert!(text.contains("points back to this ModelPort listener"));
    assert!(text.contains("MODELPORT_OPENAI_BASE_URL"));

    let unspecified_host = run_env_default_config_validate(&[
        ("MODELPORT_OPENAI_BASE_URL", "http://0.0.0.0:17878/v1"),
        ("MODELPORT_OPENAI_API_KEY", "ci-openai-provider-token"),
        ("MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS", "1"),
    ]);
    let unspecified_host_text = output_text(&unspecified_host);
    assert!(
        !unspecified_host.status.success(),
        "{unspecified_host_text}"
    );
    assert!(unspecified_host_text.contains("points back to this ModelPort listener"));
}

#[test]
fn oidc_static_preflight_is_fail_closed_and_does_not_contact_the_provider() {
    let disabled_with_explicit_safe_defaults = run_env_default_config_validate(&[
        ("MODELPORT_OIDC_AUTO_PROVISION", "0"),
        ("MODELPORT_OIDC_ALLOW_INSECURE_HTTP", "0"),
    ]);
    let disabled_text = output_text(&disabled_with_explicit_safe_defaults);
    assert!(
        disabled_with_explicit_safe_defaults.status.success(),
        "{disabled_text}"
    );

    let partial = run_env_default_config_validate(&[(
        "MODELPORT_OIDC_ISSUER",
        "https://identity.example.com/realms/modelport",
    )]);
    let partial_text = output_text(&partial);
    assert!(!partial.status.success(), "{partial_text}");
    assert!(partial_text.contains("MODELPORT_OIDC_CLIENT_ID"));

    let insecure_remote = run_env_default_config_validate(&[
        ("MODELPORT_OIDC_ISSUER", "http://identity.example.com"),
        ("MODELPORT_OIDC_CLIENT_ID", "modelport"),
        (
            "MODELPORT_OIDC_REDIRECT_URI",
            "http://modelport.example.com/admin/auth/oidc/callback",
        ),
        ("MODELPORT_OIDC_ALLOW_INSECURE_HTTP", "1"),
    ]);
    let insecure_remote_text = output_text(&insecure_remote);
    assert!(!insecure_remote.status.success(), "{insecure_remote_text}");
    assert!(insecure_remote_text.contains("must use HTTPS"));

    let missing_secure_cookie = run_env_default_config_validate(&[
        (
            "MODELPORT_OIDC_ISSUER",
            "https://identity.example.com/realms/modelport",
        ),
        ("MODELPORT_OIDC_CLIENT_ID", "modelport"),
        (
            "MODELPORT_OIDC_REDIRECT_URI",
            "https://modelport.example.com/admin/auth/oidc/callback",
        ),
    ]);
    let missing_secure_cookie_text = output_text(&missing_secure_cookie);
    assert!(
        !missing_secure_cookie.status.success(),
        "{missing_secure_cookie_text}"
    );
    assert!(missing_secure_cookie_text.contains("MODELPORT_ADMIN_COOKIE_SECURE=1"));

    let placeholder_secret = run_env_default_config_validate(&[
        (
            "MODELPORT_OIDC_ISSUER",
            "https://identity.example.com/realms/modelport",
        ),
        ("MODELPORT_OIDC_CLIENT_ID", "modelport"),
        ("MODELPORT_OIDC_CLIENT_SECRET", "replace-with-client-secret"),
        (
            "MODELPORT_OIDC_REDIRECT_URI",
            "https://modelport.example.com/admin/auth/oidc/callback",
        ),
        ("MODELPORT_ADMIN_COOKIE_SECURE", "1"),
    ]);
    let placeholder_secret_text = output_text(&placeholder_secret);
    assert!(
        !placeholder_secret.status.success(),
        "{placeholder_secret_text}"
    );
    assert!(placeholder_secret_text.contains("must not be a placeholder"));

    let valid = run_env_default_config_validate(&[
        (
            "MODELPORT_OIDC_ISSUER",
            "https://identity.example.com/realms/modelport",
        ),
        ("MODELPORT_OIDC_CLIENT_ID", "modelport"),
        (
            "MODELPORT_OIDC_REDIRECT_URI",
            "https://modelport.example.com/admin/auth/oidc/callback",
        ),
        ("MODELPORT_ADMIN_COOKIE_SECURE", "1"),
    ]);
    let valid_text = output_text(&valid);
    assert!(valid.status.success(), "{valid_text}");
    assert!(valid_text.contains("ModelPort configuration valid"));
}
