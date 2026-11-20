//! Bounded file observations. No shell commands, test execution, or raw content export.
use crate::tasks::{ArtifactCriterion, ArtifactObservation};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

pub(crate) const MAX_FILE: u64 = 4 * 1024 * 1024;
const TOTAL_BUDGET: u64 = 16 * 1024 * 1024;

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

pub(crate) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.contains('\0')
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

fn anchored_file(root: &File, path: &str) -> std::io::Result<File> {
    if !valid_path(path) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut directory = root.try_clone()?;
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let name = CString::new(component)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if components.peek().is_some() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: directory and NUL-terminated name remain valid during openat; no creation flags.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: successful openat returned a fresh owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if components.peek().is_none() {
            return Ok(file);
        }
        directory = file;
    }
    Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

fn observe_one(
    root: &File,
    criterion: &ArtifactCriterion,
    budget: &mut u64,
) -> ArtifactObservation {
    let mut observation = ArtifactObservation {
        path: criterion.path.clone(),
        status: "unavailable".into(),
        sha256: None,
        bytes: None,
    };
    let mut file = match anchored_file(root, &criterion.path) {
        Ok(file) => file,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                observation.status = "missing".into();
            }
            return observation;
        }
    };
    let Ok(before) = file.metadata() else {
        return observation;
    };
    if !before.is_file() || before.nlink() != 1 {
        observation.status = "rejected_file_type_or_link".into();
        return observation;
    }
    if before.len() > MAX_FILE {
        observation.status = "too_large".into();
        return observation;
    }
    if *budget == 0 || before.len() > *budget {
        observation.status = "budget_exceeded".into();
        return observation;
    }
    let cap = MAX_FILE.min(*budget);
    let mut contents = Vec::new();
    let read_result = (&mut file).take(cap + 1).read_to_end(&mut contents);
    *budget = budget.saturating_sub(contents.len() as u64);
    if read_result.is_err() {
        return observation;
    }
    if contents.len() as u64 > cap {
        observation.status = "read_limit_exceeded".into();
        return observation;
    }
    let Ok(after) = file.metadata() else {
        return observation;
    };
    let fingerprint = |m: &std::fs::Metadata| {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
            m.nlink(),
        )
    };
    if fingerprint(&before) != fingerprint(&after) || after.len() != contents.len() as u64 {
        observation.status = "changed_during_read".into();
        return observation;
    }
    let same_path = anchored_file(root, &criterion.path)
        .and_then(|file| file.metadata())
        .is_ok_and(|metadata| fingerprint(&metadata) == fingerprint(&after));
    if !same_path {
        observation.status = "changed_during_read".into();
        return observation;
    }
    let digest = sha256(&contents);
    observation.status = if criterion
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &digest)
    {
        "mismatched"
    } else if criterion.expected_sha256.is_some() {
        "matched"
    } else {
        "present"
    }
    .into();
    observation.sha256 = Some(digest);
    observation.bytes = Some(contents.len() as u64);
    observation
}

pub(crate) fn observe(
    repository: &Path,
    criteria: &[ArtifactCriterion],
) -> Vec<ArtifactObservation> {
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(repository);
    match root {
        Ok(root) => {
            let mut budget = TOTAL_BUDGET;
            criteria
                .iter()
                .map(|c| observe_one(&root, c, &mut budget))
                .collect()
        }
        Err(_) => criteria
            .iter()
            .map(|c| ArtifactObservation {
                path: c.path.clone(),
                status: "repository_unavailable".into(),
                sha256: None,
                bytes: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn criterion(path: &str) -> ArtifactCriterion {
        ArtifactCriterion {
            path: path.into(),
            expected_sha256: None,
        }
    }

    #[test]
    fn anchored_reads_reject_escape_symlinks_hardlinks_and_nonregular_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join("literal $(no-shell)"), b"hello world").unwrap();
        let evidence = observe(&repo, &[criterion("literal $(no-shell)")]);
        assert_eq!(evidence[0].status, "present");
        assert_eq!(
            evidence[0].sha256.as_deref(),
            Some("b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9")
        );
        std::fs::write(temp.path().join("outside"), b"outside secret").unwrap();
        std::os::unix::fs::symlink("../outside", repo.join("link")).unwrap();
        std::os::unix::fs::symlink(temp.path(), repo.join("directory-link")).unwrap();
        std::fs::hard_link(temp.path().join("outside"), repo.join("hard-link")).unwrap();
        let fifo = CString::new(repo.join("fifo").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let paths = [
            "../outside",
            "/etc/passwd",
            "link",
            "directory-link/outside",
            "hard-link",
            "fifo",
            "absent",
        ];
        let observed = observe(&repo, &paths.map(criterion));
        assert!(
            observed
                .iter()
                .all(|o| o.sha256.is_none() && o.bytes.is_none())
        );
        assert_eq!(observed.last().unwrap().status, "missing");
        let linked_root = temp.path().join("linked-root");
        std::os::unix::fs::symlink(&repo, &linked_root).unwrap();
        assert_eq!(
            observe(&linked_root, &[criterion("literal $(no-shell)")])[0].status,
            "repository_unavailable"
        );
    }

    #[test]
    fn file_and_aggregate_read_budgets_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        File::create(temp.path().join("oversized"))
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert_eq!(
            observe(temp.path(), &[criterion("oversized")])[0].status,
            "too_large"
        );
        let mut criteria = Vec::new();
        for i in 0..4 {
            let path = format!("file-{i}");
            File::create(temp.path().join(&path))
                .unwrap()
                .set_len(MAX_FILE)
                .unwrap();
            criteria.push(criterion(&path));
        }
        std::fs::write(temp.path().join("extra"), b"x").unwrap();
        criteria.push(criterion("extra"));
        let observed = observe(temp.path(), &criteria);
        assert!(observed[..4].iter().all(|o| o.status == "present"));
        assert_eq!(observed[4].status, "budget_exceeded");
    }
}
