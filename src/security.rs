use std::{
    fs::{self, OpenOptions},
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
};

use rand::{rngs::OsRng, RngCore};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("failed to read API key file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create API key file {path}: {source}")]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write API key file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("API key file {path} is empty")]
    Empty { path: PathBuf },
}

pub fn load_or_create_api_key(path: impl AsRef<Path>) -> Result<String, ApiKeyError> {
    let path = path.as_ref();

    match read_api_key(path) {
        Ok(key) => return Ok(key),
        Err(ApiKeyError::Read { source, .. }) if source.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ApiKeyError::Create {
            path: path.to_path_buf(),
            source,
        })?;
        set_private_directory_permissions(parent);
    }

    let key = generate_api_key();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_private_file_permissions(&mut options);

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(key.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .map_err(|source| ApiKeyError::Write {
                    path: path.to_path_buf(),
                    source,
                })?;
            set_private_file_permissions_on_existing(path);
            Ok(key)
        }
        Err(source) if source.kind() == ErrorKind::AlreadyExists => read_api_key(path),
        Err(source) => Err(ApiKeyError::Create {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub fn generate_and_store_api_key(path: impl AsRef<Path>) -> Result<String, ApiKeyError> {
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ApiKeyError::Create {
            path: path.to_path_buf(),
            source,
        })?;
        set_private_directory_permissions(parent);
    }

    let key = generate_api_key();
    fs::write(path, format!("{key}\n")).map_err(|source| ApiKeyError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    set_private_file_permissions_on_existing(path);
    Ok(key)
}

pub fn read_api_key(path: impl AsRef<Path>) -> Result<String, ApiKeyError> {
    let path = path.as_ref();
    let value = fs::read_to_string(path).map_err(|source| ApiKeyError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    set_private_file_permissions_on_existing(path);
    let key = value.trim();

    if key.is_empty() {
        return Err(ApiKeyError::Empty {
            path: path.to_path_buf(),
        });
    }

    Ok(key.to_owned())
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());

    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }

    difference == 0
}

fn generate_api_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);

    let mut key = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    key
}

const HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(unix)]
fn set_private_file_permissions(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_permissions(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_file_permissions_on_existing(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_private_file_permissions_on_existing(_path: &Path) {}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn generates_and_reuses_a_key() {
        let path = test_path("reuse");
        let first = load_or_create_api_key(&path).unwrap();
        let second = load_or_create_api_key(&path).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(path.exists());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn constant_time_comparison_checks_length_and_content() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"different"));
        assert!(!constant_time_eq(b"same", b"sam"));
    }

    fn test_path(name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("llm-relay-api-key-{name}-{timestamp}"))
    }
}
