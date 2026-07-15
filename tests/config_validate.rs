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
