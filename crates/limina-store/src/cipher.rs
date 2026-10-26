//! Encrypted-at-rest project secrets.
//!
//! Tokens are Fernet tokens over a compact JSON payload `{"project", "name", "value"}`, the format
//! the Python runtime wrote, so existing secrets stay readable. Binding the payload to the
//! project and name stops a ciphertext from being replayed under another resource.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;

use limina_core::{LiminaError, Result};
use serde_json::{Value, json};

pub struct SecretCipher {
    fernet: fernet::Fernet,
}

impl std::fmt::Debug for SecretCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretCipher(..)")
    }
}

impl SecretCipher {
    pub fn new(key: &str) -> Result<Self> {
        let fernet = fernet::Fernet::new(key.trim()).ok_or_else(|| {
            LiminaError::invariant("The Limina secret-encryption key is invalid.")
                .with("suggestion", "Provide a URL-safe base64 Fernet key in LIMINA_SECRET_KEY.")
        })?;
        Ok(Self { fernet })
    }

    /// Uses `LIMINA_SECRET_KEY` when set, otherwise a persistent key file created on first use.
    pub fn load(key_path: &Path) -> Result<Self> {
        match std::env::var("LIMINA_SECRET_KEY") {
            Ok(key) if !key.trim().is_empty() => Self::new(&key),
            _ => Self::new(&read_or_create_key(key_path)?),
        }
    }

    #[allow(clippy::expect_used, reason = "a freshly generated Fernet key is always valid")]
    pub fn ephemeral() -> Self {
        Self::new(&fernet::Fernet::generate_key()).expect("generated keys are valid")
    }

    pub fn generate_key() -> String {
        fernet::Fernet::generate_key()
    }

    pub fn encrypt(&self, project: &str, name: &str, value: &str) -> String {
        let payload = json!({"project": project, "name": name, "value": value});
        self.fernet.encrypt(payload.to_string().as_bytes())
    }

    pub fn decrypt(&self, project: &str, name: &str, ciphertext: &str) -> Result<String> {
        let cannot_decrypt = || {
            LiminaError::invariant(format!("Secret '{name}' cannot be decrypted."))
                .with("resource", name)
                .with(
                    "suggestion",
                    "Restore the instance secret key or replace the project secret.",
                )
        };
        let plaintext = self.fernet.decrypt(ciphertext).map_err(|_| cannot_decrypt())?;
        let payload: Value = serde_json::from_slice(&plaintext).map_err(|_| cannot_decrypt())?;
        let invalid =
            || LiminaError::invariant(format!("Secret '{name}' has an invalid encrypted payload."));
        let object = payload.as_object().ok_or_else(invalid)?;
        if object.get("project").and_then(Value::as_str) != Some(project)
            || object.get("name").and_then(Value::as_str) != Some(name)
        {
            return Err(LiminaError::invariant(format!(
                "Secret '{name}' is bound to a different project or name."
            ))
            .with("resource", name));
        }
        object.get("value").and_then(Value::as_str).map(str::to_owned).ok_or_else(invalid)
    }
}

fn read_or_create_key(path: &Path) -> Result<String> {
    let io_error = |error: std::io::Error| {
        LiminaError::invariant(format!(
            "The Limina secret key file '{}' is unusable.",
            path.display()
        ))
        .with("reason", error.to_string())
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let key = match options.open(path) {
        Ok(mut file) => {
            let key = fernet::Fernet::generate_key();
            file.write_all(format!("{key}\n").as_bytes()).map_err(io_error)?;
            key
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path).map_err(io_error)?.permissions().mode();
                if mode & 0o077 != 0 {
                    return Err(LiminaError::invariant(format!(
                        "The Limina secret key file '{}' is readable by other users.",
                        path.display()
                    ))
                    .with(
                        "suggestion",
                        format!("Run `chmod 600 {}` before starting Limina.", path.display()),
                    ));
                }
            }
            std::fs::read_to_string(path).map_err(io_error)?.trim().to_owned()
        }
        Err(error) => return Err(io_error(error)),
    };
    if key.is_empty() {
        return Err(LiminaError::invariant(format!(
            "The Limina secret key file '{}' is empty.",
            path.display()
        )));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_stay_bound_to_their_resource() {
        let cipher = SecretCipher::ephemeral();
        let token = cipher.encrypt("demo", "API_KEY", "s3cr3t");
        assert_eq!(cipher.decrypt("demo", "API_KEY", &token).unwrap(), "s3cr3t");
        assert!(cipher.decrypt("other", "API_KEY", &token).is_err());
        assert!(SecretCipher::ephemeral().decrypt("demo", "API_KEY", &token).is_err());
    }

    #[test]
    fn key_files_are_created_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys/secret.key");
        let first = read_or_create_key(&path).unwrap();
        assert_eq!(read_or_create_key(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
