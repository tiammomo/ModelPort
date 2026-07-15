use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppError,
    auth::AuthStore,
    config::{AppConfig, ConfigIssueSeverity},
    control::ControlStore,
    enterprise_ledger::EnterpriseLedger,
    storage::{JsonStore, write_json_file_atomic},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Serve,
    Help,
    ValidateConfig,
    ExportBackup(String),
    ValidateBackup(String),
    RestoreBackup(String),
}

pub(crate) fn handle(args: Vec<String>) -> Result<bool, AppError> {
    match parse_command(&args)? {
        Command::Serve => Ok(false),
        Command::Help => {
            print_usage();
            Ok(true)
        }
        Command::ValidateConfig => {
            validate_config()?;
            Ok(true)
        }
        Command::ExportBackup(path) => {
            export_backup(&path)?;
            Ok(true)
        }
        Command::ValidateBackup(path) => {
            validate_backup(&path)?;
            Ok(true)
        }
        Command::RestoreBackup(path) => {
            restore_backup(&path)?;
            Ok(true)
        }
    }
}

fn parse_command(args: &[String]) -> Result<Command, AppError> {
    match args {
        [] => Ok(Command::Serve),
        [flag] if flag == "-h" || flag == "--help" => Ok(Command::Help),
        [command, subcommand] if command == "config" && subcommand == "validate" => {
            Ok(Command::ValidateConfig)
        }
        [command, subcommand, path] if command == "backup" && subcommand == "export" => {
            Ok(Command::ExportBackup(path.clone()))
        }
        [command, subcommand, path] if command == "backup" && subcommand == "validate" => {
            Ok(Command::ValidateBackup(path.clone()))
        }
        [command, subcommand, path, flag]
            if command == "backup" && subcommand == "restore" && flag == "--yes" =>
        {
            Ok(Command::RestoreBackup(path.clone()))
        }
        _ => Err(AppError::InvalidRequest(format!(
            "unknown command `{}`; run `model-port --help`",
            args.join(" ")
        ))),
    }
}

fn validate_config() -> Result<(), AppError> {
    let config = AppConfig::load()?;
    EnterpriseLedger::validate_configuration()?;
    let issues = config.validation_issues();
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Warning)
        .count();

    println!("ModelPort configuration");
    println!("  bind: {}", config.bind_addr);
    println!("  default_provider: {}", config.default_provider);
    println!("  providers: {}", config.provider_order.join(", "));
    println!(
        "  auth: {}",
        if config.auth_token.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );

    for issue in &issues {
        let label = match issue.severity {
            ConfigIssueSeverity::Error => "ERROR",
            ConfigIssueSeverity::Warning => "WARN",
        };
        println!("[{label}] {}", issue.message);
    }

    if errors > 0 {
        return Err(AppError::Config(format!(
            "configuration validation failed with {errors} error(s) and {warnings} warning(s)"
        )));
    }

    println!("ModelPort configuration valid: {warnings} warning(s).");
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalBackupFile {
    schema_version: u32,
    service: String,
    generated_at: String,
    contains_secrets: bool,
    auth_store_path: String,
    control_store_path: String,
    auth: Value,
    control: Value,
}

fn export_backup(path: &str) -> Result<(), AppError> {
    let auth_store = JsonStore::open("auth", AuthStore::default_data_path())?;
    let control_store = JsonStore::open("control", ControlStore::default_data_path())?;
    let backup = LocalBackupFile {
        schema_version: 1,
        service: "model-port".to_owned(),
        generated_at: now_millis().to_string(),
        contains_secrets: true,
        auth_store_path: auth_store.location(),
        control_store_path: control_store.location(),
        auth: auth_store
            .read_value()?
            .unwrap_or_else(|| json!({ "users": [] })),
        control: control_store
            .read_value()?
            .unwrap_or_else(default_control_json),
    };
    write_json_file(Path::new(path), &serde_json::to_value(backup)?)?;
    println!("ModelPort backup written to {path}");
    Ok(())
}

fn validate_backup(path: &str) -> Result<(), AppError> {
    let backup = load_backup(path)?;
    let user_count = backup
        .auth
        .get("users")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let api_key_count = backup
        .control
        .get("apiKeys")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    println!(
        "ModelPort backup valid: {user_count} user(s), {api_key_count} API key record(s), contains_secrets={}",
        backup.contains_secrets
    );
    Ok(())
}

fn restore_backup(path: &str) -> Result<(), AppError> {
    let backup = load_backup(path)?;
    let auth_store = JsonStore::open("auth", AuthStore::default_data_path())?;
    let control_store = JsonStore::open("control", ControlStore::default_data_path())?;
    backup_existing_state(path, "auth", auth_store.read_value()?)?;
    backup_existing_state(path, "control", control_store.read_value()?)?;
    auth_store.write_value(&backup.auth)?;
    control_store.write_value(&backup.control)?;
    println!(
        "ModelPort backup restored to {} and {}",
        auth_store.location(),
        control_store.location()
    );
    Ok(())
}

fn load_backup(path: &str) -> Result<LocalBackupFile, AppError> {
    let raw = fs::read_to_string(path)?;
    let backup: LocalBackupFile = serde_json::from_str(&raw)?;
    if backup.schema_version != 1 || backup.service != "model-port" {
        return Err(AppError::InvalidRequest(
            "not a supported ModelPort backup".to_owned(),
        ));
    }
    if !backup.auth.get("users").is_some_and(Value::is_array) {
        return Err(AppError::InvalidRequest(
            "backup auth.users must be an array".to_owned(),
        ));
    }
    if !backup.control.is_object() {
        return Err(AppError::InvalidRequest(
            "backup control must be an object".to_owned(),
        ));
    }
    crate::auth::validate_backup_document(&backup.auth)?;
    crate::control::validate_backup_document(&backup.control)?;
    Ok(backup)
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), AppError> {
    write_json_file_atomic(path, value)
}

fn backup_existing_state(path: &str, label: &str, value: Option<Value>) -> Result<(), AppError> {
    let Some(value) = value else {
        return Ok(());
    };
    let backup_path = format!("{path}.{label}.bak.{}.json", now_millis());
    write_json_file(Path::new(&backup_path), &value)
}

fn default_control_json() -> Value {
    json!({
        "teams": [],
        "apiKeys": [],
        "quotas": [],
        "usage": [],
        "spendLedger": {},
        "routeConfig": {},
        "activities": [],
        "providerTests": [],
        "providerHealth": [],
    })
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn print_usage() {
    println!(
        "Usage:\n  model-port\n  model-port config validate\n  model-port backup export <path>\n  model-port backup validate <path>\n  model-port backup restore <path> --yes\n\nCommands:\n  config validate          Load and validate configuration without starting the server\n  backup export <path>     Export a complete local backup with hashed auth material\n  backup validate <path>   Validate a local backup file\n  backup restore <path> --yes\n                           Restore local state after backing up current files"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_server_and_read_only_commands() {
        assert_eq!(parse_command(&[]).unwrap(), Command::Serve);
        assert_eq!(parse_command(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(
            parse_command(&args(&["config", "validate"])).unwrap(),
            Command::ValidateConfig
        );
        assert_eq!(
            parse_command(&args(&["backup", "validate", "backup.json"])).unwrap(),
            Command::ValidateBackup("backup.json".to_owned())
        );
    }

    #[test]
    fn restore_requires_explicit_confirmation() {
        assert!(parse_command(&args(&["backup", "restore", "backup.json"])).is_err());
        assert_eq!(
            parse_command(&args(&["backup", "restore", "backup.json", "--yes"])).unwrap(),
            Command::RestoreBackup("backup.json".to_owned())
        );
    }
}
