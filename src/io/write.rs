//! Writing results out: stdout, and in-place replacement that keeps a file's
//! mode bits.

use super::WorkflowError;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) fn write_all_stdout(bytes: &[u8]) -> Result<(), WorkflowError> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    stdout.write_all(bytes)?;
    stdout.flush()?;
    Ok(())
}

/// Replace a file atomically, preserving its mode bits. Symlink resolution is
/// done by the caller, so a symlink argument leaves the link intact.
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), WorkflowError> {
    let target = fs::canonicalize(path)?;
    let metadata = fs::metadata(&target)?;
    let mode = mode_bits(&metadata);
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        ".{}.forformat-{}-{}",
        target.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        number
    );
    let temporary = target.parent().unwrap_or_else(|| Path::new(".")).join(name);
    let result = (|| -> Result<(), WorkflowError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        set_mode(&file, mode)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn mode_bits(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn mode_bits(_metadata: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_mode(file: &File, mode: u32) -> Result<(), WorkflowError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: u32) -> Result<(), WorkflowError> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::atomic_replace;
    use std::fs;

    #[test]
    fn atomic_replace_preserves_mode_and_cleans_failed_temporary_write() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("forformat-io-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("source.f90");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_replace(&path, b"new\n").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        let failure = atomic_replace(&directory, b"nope\n");
        assert!(failure.is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let _ = fs::remove_dir_all(directory);
    }
}
