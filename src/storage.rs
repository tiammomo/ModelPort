use std::{
    fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::PgPool;

use crate::{
    database::{connect_pool, database_url, redact_database_url},
    error::AppError,
};

const STATE_TABLE: &str = "modelport_state";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
            Self::File(path) => write_json_file_atomic(path, value),
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

pub(crate) fn write_json_file_atomic(path: &Path, value: &Value) -> Result<(), AppError> {
    let contents = serde_json::to_vec_pretty(value)?;
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;

    let (temporary_path, temporary_file) = create_secure_temporary_file(path, parent)?;
    let result = write_and_replace(temporary_file, &temporary_path, path, parent, &contents);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_secure_temporary_file(path: &Path, parent: &Path) -> io::Result<(PathBuf, File)> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "JSON store path must include a file name",
        )
    })?;

    for _ in 0..16 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_name = format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            sequence
        );
        let temporary_path = parent.join(temporary_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(&temporary_path) {
            Ok(file) => return Ok((temporary_path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary JSON store file",
    ))
}

fn write_and_replace(
    mut temporary_file: File,
    temporary_path: &Path,
    destination: &Path,
    parent: &Path,
    contents: &[u8],
) -> Result<(), AppError> {
    temporary_file.write_all(contents)?;
    temporary_file.flush()?;
    temporary_file.sync_all()?;
    drop(temporary_file);

    fs::rename(temporary_path, destination)?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

async fn initialize_postgres(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {STATE_TABLE} (
            namespace TEXT PRIMARY KEY,
            document JSONB NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"
    ))
    .execute(pool)
    .await?;
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
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready_sender
                        .send(Err(format!("could not create PostgreSQL runtime: {err}")));
                    return;
                }
            };
            let pool = match runtime.block_on(connect_and_initialize(
                &database_url,
                &namespace,
                &file_path,
            )) {
                Ok(pool) => {
                    let _ = ready_sender.send(Ok(()));
                    pool
                }
                Err(err) => {
                    let _ = ready_sender.send(Err(err.to_string()));
                    return;
                }
            };

            for command in command_receiver {
                match command {
                    PostgresCommand::Read { respond_to } => {
                        let result = runtime
                            .block_on(read_postgres_value(&pool, &namespace))
                            .map_err(|err| err.to_string());
                        let _ = respond_to.send(result);
                    }
                    PostgresCommand::Write { value, respond_to } => {
                        let result = runtime
                            .block_on(write_postgres_value(&pool, &namespace, &value))
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

async fn connect_and_initialize(
    database_url: &str,
    namespace: &str,
    file_path: &Path,
) -> Result<PgPool, AppError> {
    let pool = connect_pool(database_url, Some(1), false).await?;
    initialize_postgres(&pool).await?;
    import_file_if_empty(&pool, namespace, file_path).await?;
    Ok(pool)
}

async fn read_postgres_value(pool: &PgPool, namespace: &str) -> Result<Option<Value>, AppError> {
    sqlx::query_scalar::<_, Value>(&format!(
        "SELECT document FROM {STATE_TABLE} WHERE namespace = $1"
    ))
    .bind(namespace)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

async fn write_postgres_value(
    pool: &PgPool,
    namespace: &str,
    value: &Value,
) -> Result<(), AppError> {
    sqlx::query(&format!(
        "INSERT INTO {STATE_TABLE} (namespace, document, updated_at) \
             VALUES ($1, $2, now()) \
             ON CONFLICT (namespace) DO UPDATE \
             SET document = EXCLUDED.document, updated_at = now()"
    ))
    .bind(namespace)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

async fn import_file_if_empty(
    pool: &PgPool,
    namespace: &str,
    file_path: &Path,
) -> Result<(), AppError> {
    let exists =
        sqlx::query_scalar::<_, i32>(&format!("SELECT 1 FROM {STATE_TABLE} WHERE namespace = $1"))
            .bind(namespace)
            .fetch_optional(pool)
            .await?
            .is_some();
    if exists || !file_path.exists() {
        return Ok(());
    }

    let value: Value = serde_json::from_str(&fs::read_to_string(file_path)?)?;
    sqlx::query(&format!(
        "INSERT INTO {STATE_TABLE} (namespace, document, updated_at) VALUES ($1, $2, now())"
    ))
    .bind(namespace)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temporary_test_directory(label: &str) -> PathBuf {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "modelport-storage-{label}-{}-{sequence}",
            std::process::id()
        ))
    }

    #[test]
    fn atomic_json_write_replaces_content_without_leaving_temporary_files() {
        let directory = temporary_test_directory("atomic");
        let path = directory.join("state.json");

        write_json_file_atomic(&path, &json!({ "version": 1 })).unwrap();
        write_json_file_atomic(&path, &json!({ "version": 2 })).unwrap();

        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored, json!({ "version": 2 }));
        let entries = fs::read_dir(&directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path(), path);

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_json_write_enforces_owner_only_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_test_directory("permissions");
        let path = directory.join("state.json");
        write_json_file_atomic(&path, &json!({ "secure": true })).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_json_file_atomic(&path, &json!({ "secure": "replaced" })).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        fs::remove_dir_all(directory).unwrap();
    }
}
