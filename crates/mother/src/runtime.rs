use crate::{
    Error, Result,
    config::{self, Config},
    protocol::{MAX_FRAME, PROTOCOL_VERSION, Request, Response},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::JoinSet,
    time::timeout,
};

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn read_frame(stream: &mut UnixStream, max: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(stream.take((max + 1) as u64));
    timeout(IO_TIMEOUT, reader.read_until(b'\n', &mut bytes))
        .await
        .map_err(|_| Error::Unavailable("IPC read timed out".into()))??;
    if bytes.len() > max || bytes.last() != Some(&b'\n') {
        return Err(Error::Invalid(
            "IPC frame is too large or missing newline".into(),
        ));
    }
    Ok(bytes)
}

async fn write_frame(stream: &mut UnixStream, value: &impl serde::Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() >= MAX_RESPONSE {
        return Err(Error::Invalid("response exceeds IPC limit".into()));
    }
    bytes.push(b'\n');
    timeout(IO_TIMEOUT, stream.write_all(&bytes))
        .await
        .map_err(|_| Error::Unavailable("IPC write timed out".into()))??;
    Ok(())
}

pub async fn call(dir: &Path, request: &Request) -> Result<Response> {
    config::check_private_dir(dir)?;
    let mut stream = timeout(IO_TIMEOUT, UnixStream::connect(dir.join("runtime.sock")))
        .await
        .map_err(|_| Error::Unavailable("runtime connection timed out".into()))?
        .map_err(|e| {
            Error::Unavailable(format!(
                "cannot connect to runtime: {e}; run motherctl serve"
            ))
        })?;
    if serde_json::to_vec(request)?.len() >= MAX_FRAME {
        return Err(Error::Invalid("request exceeds IPC limit".into()));
    }
    write_frame(&mut stream, request).await?;
    let response: Response = serde_json::from_slice(&read_frame(&mut stream, MAX_RESPONSE).await?)?;
    if response.version != PROTOCOL_VERSION || response.request_id != request.request_id {
        return Err(Error::Invalid(
            "runtime response version or correlation mismatch".into(),
        ));
    }
    Ok(response)
}

fn error_response(id: String, error: Error) -> Response {
    let (code, message) = match error {
        Error::Invalid(s) => ("invalid_request", s),
        Error::Conflict(s) => ("conflict", s),
        Error::NotFound(s) => ("not_found", s),
        Error::Integrity(s) => ("integrity_error", s),
        _ => {
            tracing::error!("local storage or I/O operation failed; request content omitted");
            (
                "internal_error",
                "local storage or I/O operation failed; run motherctl doctor".into(),
            )
        }
    };
    Response::failure(id, code, &message)
}

async fn connection(mut stream: UnixStream, store: Arc<Mutex<Store>>) -> Result<()> {
    let frame = match read_frame(&mut stream, MAX_FRAME).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return write_frame(
                &mut stream,
                &Response::failure(
                    String::new(),
                    "invalid_request",
                    "invalid or timed out IPC frame",
                ),
            )
            .await;
        }
    };
    let request: Request = match serde_json::from_slice(&frame) {
        Ok(value) => value,
        Err(_) => {
            return write_frame(
                &mut stream,
                &Response::failure(String::new(), "invalid_request", "invalid request schema"),
            )
            .await;
        }
    };
    let id = request.request_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| Error::Unavailable("database owner failed".into()))?
            .execute(&request)
    })
    .await
    .map_err(|_| Error::Unavailable("database operation failed".into()))?;
    let response = match result {
        Ok(value) => Response::success(id, value),
        Err(error) => error_response(id, error),
    };
    write_frame(&mut stream, &response).await
}

struct SocketGuard(std::path::PathBuf);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub async fn serve(dir: &Path) -> Result<()> {
    // Lock precedes database access and stale socket removal. Never unlink the lock file.
    let _lock = config::lock(dir)?;
    let config = Config::load(dir)?;
    let path = dir.join("runtime.sock");
    if path.as_os_str().as_encoded_bytes().len() > 100 {
        return Err(Error::Invalid(
            "socket path exceeds portable 100-byte limit; use a shorter --data-dir".into(),
        ));
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&path)?,
        Ok(_) => {
            return Err(Error::Invalid(
                "runtime.sock exists and is not a socket".into(),
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let store = Arc::new(Mutex::new(Store::open(dir)?));
    let listener = UnixListener::bind(&path)?;
    let _socket = SocketGuard(path.clone());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    tracing::info!("Mother runtime ready");
    let semaphore = Arc::new(Semaphore::new(config.max_clients));
    let mut clients = JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let shutdown = async {
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    };
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            Some(result) = clients.join_next(), if !clients.is_empty() => {
                if !matches!(result, Ok(Ok(()))) { tracing::debug!("client disconnected or request failed"); }
            },
            incoming = listener.accept() => {
                let (stream,_) = incoming?;
                // Reject excess connections without allocating another request task.
                if let Ok(permit) = semaphore.clone().try_acquire_owned() {
                    let store = store.clone();
                    clients.spawn(async move { let _permit = permit; connection(stream,store).await });
                }
            }
        }
    }
    drop(listener);
    while clients.join_next().await.is_some() {}
    tracing::info!("Mother runtime stopped");
    Ok(())
}

pub async fn doctor(dir: &Path) -> Value {
    let mut checks = Vec::new();
    let private = config::check_private_dir(dir);
    checks.push(json!({"check":"private_data_directory","ok":private.is_ok(),"detail":private.err().map(|e|e.to_string())}));
    if checks[0]["ok"] == false {
        return json!({"ok":false,"checks":checks,"next_action":"run motherctl init, or fix directory ownership and permissions"});
    }
    let configuration = Config::load(dir);
    checks.push(json!({"check":"configuration","ok":configuration.is_ok(),"detail":configuration.err().map(|e|e.to_string())}));
    let database = (|| -> Result<Value> {
        let path = dir.join("mother.db");
        config::read_private_file(&path)?;
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        let foreign_key_errors: i64 =
            connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        Ok(
            json!({"check":"database","ok":version == crate::store::SCHEMA_VERSION && integrity == "ok" && foreign_key_errors == 0,"schema_version":version,"integrity":integrity,"foreign_key_errors":foreign_key_errors}),
        )
    })();
    checks.push(
        database.unwrap_or_else(|e| json!({"check":"database","ok":false,"detail":e.to_string()})),
    );
    let request = Request {
        version: PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        operation: crate::protocol::Operation::Health,
    };
    let health = call(dir, &request).await;
    checks.push(match health {
        Ok(response) => {
            json!({"check":"runtime","ok":response.error.is_none(),"health":response.result})
        }
        Err(e) => json!({"check":"runtime","ok":false,"detail":e.to_string()}),
    });
    let ok = checks.iter().all(|v| v["ok"] == true);
    json!({"ok":ok,"checks":checks})
}
