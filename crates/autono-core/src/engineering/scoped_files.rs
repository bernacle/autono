//! Descriptor-relative file operations for delegated edits and prompt reads.
//! A sandboxed check must not turn a symlink race into an unsandboxed daemon
//! read/write. Never follow a path after checking it, or overwrite a shared inode.
#[cfg(unix)]
mod unix {
    use anyhow::{Context, Result, ensure};
    use std::{
        ffi::CString,
        fs::{File, OpenOptions, Permissions},
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
        path::Path,
    };

    fn open_at(
        parent: &File,
        name: &CString,
        flags: i32,
        mode: libc::mode_t,
    ) -> std::io::Result<File> {
        // SAFETY: the descriptor is live, CString is terminated, and ownership of
        // a successful new descriptor is transferred to File exactly once.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn parent(cwd: &Path, name: &str, create: bool) -> std::io::Result<(File, CString)> {
        if name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid scoped file path",
            ));
        }
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(cwd)?;
        let parts: Vec<_> = name.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            let part = CString::new(*part)?;
            let opened = open_at(&directory, &part, libc::O_RDONLY | libc::O_DIRECTORY, 0);
            directory = match opened {
                Ok(file) => file,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    // SAFETY: directory and CString remain live for the call.
                    let result =
                        unsafe { libc::mkdirat(directory.as_raw_fd(), part.as_ptr(), 0o755) };
                    if result < 0
                        && std::io::Error::last_os_error().kind()
                            != std::io::ErrorKind::AlreadyExists
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    // A racing replacement must still be a real directory.
                    open_at(&directory, &part, libc::O_RDONLY | libc::O_DIRECTORY, 0)?
                }
                Err(error) => return Err(error),
            };
            // Never traverse a nested repository/submodule, including one
            // introduced after discovery. Do not follow its .git marker.
            let marker = CString::new(".git")?;
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: live directory, terminated name and valid output storage.
            let found = unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    marker.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if found == 0 {
                return Err(std::io::Error::other(
                    "nested repositories are outside source authority",
                ));
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok((
            directory,
            CString::new(
                *parts
                    .last()
                    .ok_or_else(|| std::io::Error::other("missing file name"))?,
            )?,
        ))
    }

    fn existing(parent: &File, name: &CString) -> Result<Option<File>> {
        let file = match open_at(parent, name, libc::O_RDONLY | libc::O_NONBLOCK, 0) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "delegated target must be an unshared regular file"
        );
        Ok(Some(file))
    }

    pub(super) fn validate(cwd: &Path, name: &str) -> Result<()> {
        let (directory, name) = match parent(cwd, name, false) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        existing(&directory, &name)?;
        Ok(())
    }

    pub(super) fn read(cwd: &Path, name: &str) -> Result<Option<String>> {
        let (directory, name) = match parent(cwd, name, false) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let Some(file) = existing(&directory, &name)? else {
            return Ok(None);
        };
        ensure!(
            file.metadata()?.len() <= 256 * 1024,
            "authorized source exceeds 256 KiB"
        );
        let mut value = String::new();
        file.take(256 * 1024 + 1)
            .read_to_string(&mut value)
            .context("authorized source must be UTF-8")?;
        ensure!(
            value.len() <= 256 * 1024,
            "authorized source grew beyond 256 KiB"
        );
        Ok(Some(value))
    }

    pub(super) fn replace(cwd: &Path, name: &str, content: &str, create_only: bool) -> Result<()> {
        let (directory, name) = parent(cwd, name, true)?;
        let mode = existing(&directory, &name)?
            .map(|f| f.metadata().map(|m| m.permissions().mode() & 0o777))
            .transpose()?
            .unwrap_or(0o644);
        let temporary = CString::new(format!(".autono-edit-{}", uuid::Uuid::new_v4()))?;
        let mut file = open_at(
            &directory,
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let result = (|| {
            file.write_all(content.as_bytes())?;
            file.set_permissions(Permissions::from_mode(mode))?;
            file.sync_all()?;
            // Replace the directory entry, never truncate/follow the prior inode.
            // SAFETY: both names and the retained parent descriptor are live.
            let renamed = if create_only {
                // Atomic no-clobber publication: a newly appeared user file is
                // never replaced between validation and application.
                let linked = unsafe {
                    libc::linkat(
                        directory.as_raw_fd(),
                        temporary.as_ptr(),
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        0,
                    )
                };
                if linked == 0 {
                    unsafe { libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0) };
                }
                linked
            } else {
                unsafe {
                    libc::renameat(
                        directory.as_raw_fd(),
                        temporary.as_ptr(),
                        directory.as_raw_fd(),
                        name.as_ptr(),
                    )
                }
            };
            ensure!(
                renamed == 0,
                "publish delegated file replacement: {}",
                std::io::Error::last_os_error()
            );
            directory.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            // SAFETY: unlinkat removes only this directory entry, never its target.
            let _ = unsafe { libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0) };
        }
        result
    }
}

pub(super) fn validate(cwd: &std::path::Path, name: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        unix::validate(cwd, name)
    }
    #[cfg(not(unix))]
    {
        let _ = (cwd, name);
        anyhow::bail!("scoped files unsupported on this OS")
    }
}
pub(super) fn read(cwd: &std::path::Path, name: &str) -> anyhow::Result<Option<String>> {
    #[cfg(unix)]
    {
        unix::read(cwd, name)
    }
    #[cfg(not(unix))]
    {
        let _ = (cwd, name);
        anyhow::bail!("scoped files unsupported on this OS")
    }
}
pub(super) fn replace(cwd: &std::path::Path, name: &str, content: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        unix::replace(cwd, name, content, false)
    }
    #[cfg(not(unix))]
    {
        let _ = (cwd, name, content);
        anyhow::bail!("scoped files unsupported on this OS")
    }
}
pub(super) fn create(cwd: &std::path::Path, name: &str, content: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        unix::replace(cwd, name, content, true)
    }
    #[cfg(not(unix))]
    {
        let _ = (cwd, name, content);
        anyhow::bail!("scoped files unsupported on this OS")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[test]
    fn reads_and_replacements_reject_symlinks_hardlinks_and_path_escapes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path())?;
        let cwd = root.join("checkout");
        let outside = root.join("outside");
        std::fs::create_dir(&cwd)?;
        std::fs::create_dir(&outside)?;
        std::fs::write(outside.join("secret"), "unchanged")?;
        symlink(&outside, cwd.join("src"))?;
        assert!(read(&cwd, "src/secret").is_err());
        assert!(replace(&cwd, "src/secret", "bad").is_err());
        std::fs::remove_file(cwd.join("src"))?;
        std::fs::create_dir(cwd.join("src"))?;
        symlink(outside.join("secret"), cwd.join("src/secret"))?;
        assert!(read(&cwd, "src/secret").is_err());
        assert!(replace(&cwd, "src/secret", "bad").is_err());
        std::fs::remove_file(cwd.join("src/secret"))?;
        std::fs::hard_link(outside.join("secret"), cwd.join("src/secret"))?;
        assert!(read(&cwd, "src/secret").is_err());
        assert!(replace(&cwd, "src/secret", "bad").is_err());
        assert!(replace(&cwd, "../outside/secret", "bad").is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("secret"))?,
            "unchanged"
        );
        replace(&cwd, "new/nested/source.sh", "printf safe")?;
        assert_eq!(
            read(&cwd, "new/nested/source.sh")?,
            Some("printf safe".into())
        );
        Ok(())
    }

    #[test]
    fn directory_link_races_cannot_escape_the_retained_worktree_descriptors() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path())?;
        let cwd = root.join("checkout");
        let outside = root.join("outside");
        std::fs::create_dir(&cwd)?;
        std::fs::create_dir(&outside)?;
        std::fs::create_dir(cwd.join("src"))?;
        std::fs::write(outside.join("secret"), "outside-canary")?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let racer_cwd = cwd.clone();
        let racer_outside = outside.clone();
        let racer = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                if std::fs::rename(racer_cwd.join("src"), racer_cwd.join("parked")).is_ok() {
                    let _ = symlink(&racer_outside, racer_cwd.join("src"));
                    std::thread::yield_now();
                    let _ = std::fs::remove_file(racer_cwd.join("src"));
                    let _ = std::fs::rename(racer_cwd.join("parked"), racer_cwd.join("src"));
                }
            }
        });
        let mut leaked = false;
        for _ in 0..128 {
            let _ = replace(&cwd, "src/secret", "inside-only");
            leaked |= read(&cwd, "src/secret").ok().flatten().as_deref() == Some("outside-canary");
        }
        stop.store(true, Ordering::Release);
        racer.join().expect("race thread");
        assert!(
            !leaked,
            "a delegated prompt read followed a racing link outside the worktree"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("secret"))?,
            "outside-canary"
        );
        Ok(())
    }
}
