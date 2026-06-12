use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    thread,
};

use postgres::{Client, NoTls};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::error::AppError;

const STATE_TABLE: &str = "modelport_state";

#[derive(Debug)]
pub enum JsonStore {
    File(PathBuf),
    Postgres(PostgresJsonStore),
}

pub struct PostgresJsonStore {
    namespace: String,
    location: String,
    worker: Mutex<mpsc::Sender<PostgresCommand>>,
}

enum PostgresCommand {
    Read {
        respond_to: mpsc::Sender<Result<Option<Value>, String>>,
    },
    Write {
        value: Value,
        respond_to: mpsc::Sender<Result<(), String>>,
    },
}

impl std::fmt::Debug for PostgresJsonStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresJsonStore")
            .field("namespace", &self.namespace)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl JsonStore {
    pub fn open(namespace: &str, file_path: PathBuf) -> Result<Self, AppError> {
        let Some(database_url) = database_url() else {
            return Ok(Self::File(file_path));
        };

        let worker = spawn_postgres_worker(database_url.clone(), namespace.to_owned(), file_path)?;

        Ok(Self::Postgres(PostgresJsonStore {
            namespace: namespace.to_owned(),
            location: format!(
                "{}#{}:{}",
                redact_database_url(&database_url),
                STATE_TABLE,
                namespace
            ),
            worker: Mutex::new(worker),
        }))
    }

    pub fn read_or_default<T>(&self, default: Value) -> Result<T, AppError>
    where
        T: DeserializeOwned,
    {
        let value = self.read_value()?.unwrap_or(default);
        Ok(serde_json::from_value(value)?)
    }

    pub fn read_value(&self) -> Result<Option<Value>, AppError> {
        match self {
            Self::File(path) => {
                if !path.exists() {
                    return Ok(None);
                }
                let value = serde_json::from_str(&fs::read_to_string(path)?)?;
                Ok(Some(value))
            }
            Self::Postgres(store) => {
                let (respond_to, response) = mpsc::channel();
                store
                    .worker
                    .lock()
                    .expect("postgres worker lock poisoned")
                    .send(PostgresCommand::Read { respond_to })
                    .map_err(|err| AppError::Database(format!("postgres worker stopped: {err}")))?;
                response
                    .recv()
                    .map_err(|err| AppError::Database(format!("postgres worker stopped: {err}")))?
                    .map_err(AppError::Database)
            }
        }
    }

    pub fn write_json<T>(&self, value: &T) -> Result<(), AppError>
    where
        T: Serialize,
    {
        self.write_value(&serde_json::to_value(value)?)
    }

    pub fn write_value(&self, value: &Value) -> Result<(), AppError> {
        match self {
            Self::File(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let tmp_path = path.with_extension("json.tmp");
                fs::write(&tmp_path, serde_json::to_string_pretty(value)?)?;
                fs::rename(tmp_path, path)?;
                Ok(())
            }
            Self::Postgres(store) => {
                let (respond_to, response) = mpsc::channel();
                store
                    .worker
                    .lock()
                    .expect("postgres worker lock poisoned")
                    .send(PostgresCommand::Write {
                        value: value.clone(),
                        respond_to,
                    })
                    .map_err(|err| AppError::Database(format!("postgres worker stopped: {err}")))?;
                response
                    .recv()
                    .map_err(|err| AppError::Database(format!("postgres worker stopped: {err}")))?
                    .map_err(AppError::Database)
            }
        }
    }

    pub fn location(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Postgres(store) => store.location.clone(),
        }
    }
}

fn database_url() -> Option<String> {
    env::var("MODELPORT_DATABASE_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn initialize_postgres(client: &mut Client) -> Result<(), AppError> {
    client.batch_execute(&format!(
        "CREATE TABLE IF NOT EXISTS {STATE_TABLE} (
            namespace TEXT PRIMARY KEY,
            document JSONB NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"
    ))?;
    Ok(())
}

fn spawn_postgres_worker(
    database_url: String,
    namespace: String,
    file_path: PathBuf,
) -> Result<mpsc::Sender<PostgresCommand>, AppError> {
    let (command_sender, command_receiver) = mpsc::channel::<PostgresCommand>();
    let (ready_sender, ready_receiver) = mpsc::channel::<Result<(), String>>();
    let thread_name = format!("modelport-postgres-{namespace}");
    thread::Builder::new().name(thread_name).spawn({
        let namespace = namespace.clone();
        move || {
            let mut client = match connect_and_initialize(&database_url, &namespace, &file_path) {
                Ok(client) => {
                    let _ = ready_sender.send(Ok(()));
                    client
                }
                Err(err) => {
                    let _ = ready_sender.send(Err(err.to_string()));
                    return;
                }
            };

            for command in command_receiver {
                match command {
                    PostgresCommand::Read { respond_to } => {
                        let result = read_postgres_value(&mut client, &namespace)
                            .map_err(|err| err.to_string());
                        let _ = respond_to.send(result);
                    }
                    PostgresCommand::Write { value, respond_to } => {
                        let result = write_postgres_value(&mut client, &namespace, &value)
                            .map_err(|err| err.to_string());
                        let _ = respond_to.send(result);
                    }
                }
            }
        }
    })?;

    ready_receiver
        .recv()
        .map_err(|err| AppError::Database(format!("postgres worker failed to start: {err}")))?
        .map_err(AppError::Database)?;

    Ok(command_sender)
}

fn connect_and_initialize(
    database_url: &str,
    namespace: &str,
    file_path: &Path,
) -> Result<Client, AppError> {
    let mut client = Client::connect(database_url, NoTls)?;
    initialize_postgres(&mut client)?;
    import_file_if_empty(&mut client, namespace, file_path)?;
    Ok(client)
}

fn read_postgres_value(client: &mut Client, namespace: &str) -> Result<Option<Value>, AppError> {
    let row = client.query_opt(
        &format!("SELECT document FROM {STATE_TABLE} WHERE namespace = $1"),
        &[&namespace],
    )?;
    Ok(row.map(|row| row.get(0)))
}

fn write_postgres_value(
    client: &mut Client,
    namespace: &str,
    value: &Value,
) -> Result<(), AppError> {
    client.execute(
        &format!(
            "INSERT INTO {STATE_TABLE} (namespace, document, updated_at) \
             VALUES ($1, $2, now()) \
             ON CONFLICT (namespace) DO UPDATE \
             SET document = EXCLUDED.document, updated_at = now()"
        ),
        &[&namespace, value],
    )?;
    Ok(())
}

fn import_file_if_empty(
    client: &mut Client,
    namespace: &str,
    file_path: &Path,
) -> Result<(), AppError> {
    let exists = client
        .query_opt(
            &format!("SELECT 1 FROM {STATE_TABLE} WHERE namespace = $1"),
            &[&namespace],
        )?
        .is_some();
    if exists || !file_path.exists() {
        return Ok(());
    }

    let value: Value = serde_json::from_str(&fs::read_to_string(file_path)?)?;
    client.execute(
        &format!(
            "INSERT INTO {STATE_TABLE} (namespace, document, updated_at) VALUES ($1, $2, now())"
        ),
        &[&namespace, &value],
    )?;
    Ok(())
}

fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "postgres://<redacted>".to_owned();
    };
    let Some((userinfo, host)) = rest.split_once('@') else {
        return format!("{scheme}://{rest}");
    };
    let username = userinfo.split(':').next().unwrap_or("modelport");
    format!("{scheme}://{username}:<redacted>@{host}")
}
