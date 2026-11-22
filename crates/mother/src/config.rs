use crate::{Error, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub format_version: u32,
    pub max_clients: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            format_version: 1,
            max_clients: 32,
        }
    }
}

impl Config {
    pub fn load(dir: &Path) -> Result<Self> {
        use std::io::Read;
        let mut contents = String::new();
        let file = read_private_file(&dir.join("config.toml"))?;
        file.take(8193).read_to_string(&mut contents)?;
        if contents.len() > 8192 {
            return Err(Error::Invalid("configuration exceeds 8192 bytes".into()));
        }
        let config: Self = toml::from_str(&contents)?;
        if config.format_version != 1 || !(1..=128).contains(&config.max_clients) {
            return Err(Error::Invalid(
                "config requires format_version = 1 and max_clients in 1..=128".into(),
            ));
        }
        Ok(config)
    }
}

pub fn default_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".mother"))
        .ok_or_else(|| Error::Invalid("HOME is unset; specify --data-dir".into()))
}

pub fn check_private_dir(dir: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(dir)?;
    // Filesystem access is the local trust boundary; same-UID programs are trusted.
    let uid = unsafe { libc::geteuid() };
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(Error::Invalid(format!(
            "{} must be a real directory owned by the current user with mode 0700",
            dir.display()
        )));
    }
    Ok(())
}

pub fn private_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate_file(path, &file)?;
    Ok(file)
}

pub fn read_private_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate_file(path, &file)?;
    Ok(file)
}

fn validate_file(path: &Path, file: &File) -> Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
    {
        return Err(Error::Invalid(format!(
            "{} must be a private, singly linked regular file",
            path.display()
        )));
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    match fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
    {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    check_private_dir(path)
}

pub fn github_profile(dir: &Path, profile_id: &str) -> Result<PathBuf> {
    check_private_dir(dir)?;
    if !crate::github::valid_profile_id(profile_id) {
        return Err(Error::Invalid(
            "profile_id must be a canonical lowercase UUID".into(),
        ));
    }
    let root = dir.join("github").join("profiles");
    create_private_dir(&root)?;
    let path = root.join(profile_id);
    create_private_dir(&path)?;
    Ok(path)
}

pub fn worker_execution_dir(dir: &Path, execution_id: &str) -> Result<PathBuf> {
    check_private_dir(dir)?;
    let valid = uuid::Uuid::parse_str(execution_id).is_ok_and(|id| id.to_string() == execution_id);
    if !valid {
        return Err(Error::Invalid(
            "execution_id must be a canonical lowercase UUID".into(),
        ));
    }
    let root = dir.join("workers");
    create_private_dir(&root)?;
    let path = root.join(execution_id);
    create_private_dir(&path)?;
    Ok(path)
}

pub fn existing_github_profile(dir: &Path, profile_id: &str) -> Result<PathBuf> {
    check_private_dir(dir)?;
    if !crate::github::valid_profile_id(profile_id) {
        return Err(Error::Invalid(
            "profile_id must be a canonical lowercase UUID".into(),
        ));
    }
    let path = dir.join("github").join("profiles").join(profile_id);
    check_private_dir(&path)?;
    Ok(path)
}

pub fn check_github_profiles(dir: &Path) -> Result<()> {
    let root = dir.join("github").join("profiles");
    match fs::symlink_metadata(&root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    check_private_dir(&root)?;
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        check_private_dir(&entry.path())?;
    }
    Ok(())
}

pub fn lock(dir: &Path) -> Result<File> {
    check_private_dir(dir)?;
    let file = private_file(&dir.join("runtime.lock"))?;
    file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Error::Conflict("runtime is already running or initialization is in progress".into())
        } else {
            Error::Io(error)
        }
    })?;
    Ok(file)
}

pub fn initialize(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let _lock = lock(dir)?;
    let path = dir.join("config.toml");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(b"format_version = 1\nmax_clients = 32\n")?;
            file.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    Config::load(dir)?;
    crate::store::Store::open(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unknown_fields_versions_and_invalid_limits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            dir.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        initialize(dir.path()).unwrap();
        for value in [
            "format_version=2\nmax_clients=32",
            "format_version=1\nmax_clients=0",
            "format_version=1\nmax_clients=129",
            "format_version=1\nmax_clients=32\ntypo=true",
        ] {
            fs::write(dir.path().join("config.toml"), value).unwrap();
            assert!(Config::load(dir.path()).is_err());
        }
    }

    #[test]
    fn github_profiles_are_private_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            dir.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        initialize(dir.path()).unwrap();
        let profile_id = uuid::Uuid::new_v4().to_string();
        let profile = github_profile(dir.path(), &profile_id).unwrap();
        assert!(profile.is_dir());
        assert_eq!(
            existing_github_profile(dir.path(), &profile_id).unwrap(),
            profile
        );
        assert!(check_github_profiles(dir.path()).is_ok());
        assert!(github_profile(dir.path(), "not-a-uuid").is_err());
        std::fs::set_permissions(
            &profile,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .unwrap();
        assert!(check_github_profiles(dir.path()).is_err());
    }

    #[test]
    fn rejects_symlink_config_and_database() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            dir.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        initialize(dir.path()).unwrap();
        fs::rename(dir.path().join("config.toml"), dir.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink("elsewhere", dir.path().join("config.toml")).unwrap();
        assert!(Config::load(dir.path()).is_err());
        fs::rename(dir.path().join("mother.db"), dir.path().join("database")).unwrap();
        std::os::unix::fs::symlink("database", dir.path().join("mother.db")).unwrap();
        assert!(crate::store::Store::open(dir.path()).is_err());
    }
}
