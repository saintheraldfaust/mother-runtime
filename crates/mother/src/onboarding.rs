//! Owner-invoked local setup; never reads or copies provider credentials.
use mother_runtime::{Error, Result, config, protocol::Operation};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub async fn start(dir: &Path) -> Result<()> {
    if let Ok(health) = super::request_result(dir, Operation::Health).await {
        if health["schema_version"].as_u64()
            != Some(u64::from(mother_runtime::store::SCHEMA_VERSION))
        {
            return Err(Error::Unavailable("running service uses a different schema; stop that service and restart it with this motherctl serve before retrying".into()));
        }
        return Ok(());
    }
    if !dir.join("config.toml").exists() {
        config::initialize(dir)?;
    }
    config::Config::load(dir)?;
    let mut log = config::private_file(&dir.join("service.log"))?;
    log.seek(SeekFrom::End(0))?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--data-dir")
        .arg(dir)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if super::request_result(dir, Operation::Health).await.is_ok() {
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            return Err(Error::Unavailable(
                "service exited; inspect private service.log or run motherctl serve".into(),
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Unavailable("service startup timed out".into()));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn setup_codex(dir: &Path) -> Result<()> {
    start(dir).await?;
    let project = std::env::current_dir()?.canonicalize()?;
    let folder = project.join(".codex");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&folder)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Ok(())
            } else {
                Err(e)
            }
        })?;
    config::check_private_dir(&folder)?;
    let path = folder.join("config.toml");
    let mut file = config::private_file(&path)?;
    let mut contents = String::new();
    (&mut file).take(65537).read_to_string(&mut contents)?;
    if contents.len() > 65536 {
        return Err(Error::Invalid("project Codex config exceeds 64 KiB".into()));
    }
    let existing: toml::Value = toml::from_str(&contents)?;
    let command = std::env::current_exe()?.canonicalize()?;
    let dir = dir.canonicalize()?;
    let args = vec![
        "--data-dir".to_owned(),
        dir.to_string_lossy().into_owned(),
        "mcp".into(),
    ];
    if let Some(mother) = existing.get("mcp_servers").and_then(|v| v.get("mother")) {
        let same = mother.get("command").and_then(toml::Value::as_str) == command.to_str()
            && mother
                .get("args")
                .and_then(toml::Value::as_array)
                .is_some_and(|a| {
                    a.iter()
                        .filter_map(toml::Value::as_str)
                        .eq(args.iter().map(String::as_str))
                });
        if !same {
            return Err(Error::Conflict("existing mcp_servers.mother differs; preserve it and reconcile project config explicitly".into()));
        }
    } else {
        // Append a new table, preserving all existing comments/settings. Validate before writing.
        let addition = format!(
            "\n[mcp_servers.mother]\ncommand = {}\nargs = {}\nrequired = true\n",
            serde_json::to_string(&command.to_string_lossy())?,
            serde_json::to_string(&args)?
        );
        let updated = format!("{contents}{addition}");
        let _: toml::Value = toml::from_str(&updated)?;
        file.write_all(addition.as_bytes())?;
        file.sync_all()?;
    }
    // Registration is a canonical IPC mutation, never a client database write.
    let mut after = 0;
    loop {
        let page =
            super::request_result(&dir, Operation::ProjectsList { after, limit: 100 }).await?;
        if page["items"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|p| p["repository"].as_str() == project.to_str())
        }) {
            break;
        }
        match page["next_cursor"].as_u64() {
            Some(next) => after = next,
            None => {
                super::request_result(
                    &dir,
                    Operation::ProjectAttach {
                        name: project
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        repository: project.to_string_lossy().into_owned(),
                    },
                )
                .await?;
                break;
            }
        }
    }
    println!(
        "Mother MCP configured for this project. Reload the Codex VS Code session in this trusted project.\nOpen the dashboard: {} ui",
        command.display()
    );
    Ok(())
}
