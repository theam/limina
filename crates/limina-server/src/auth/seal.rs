//! Sealed cookie payloads, PKCE and random tokens.
//!
//! # Sealed value format (stable; shared with the Python runtime)
//!
//! ```text
//! key    = HKDF-SHA256(ikm = UTF-8 cookie password, salt = "limina-workos", info = purpose, L = 32)
//! sealed = 0x01 || nonce (12 random bytes) || AES-256-GCM(key, nonce, plaintext, aad = 0x01)
//! value  = base64url(sealed) without padding
//! ```
//!
//! The plaintext is a UTF-8 JSON object. The purpose (`session` or `login-flow`) gives each cookie
//! its own key, so one cannot be replayed as the other. The leading byte versions the format.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use base64::engine::{DecodePaddingMode, general_purpose};
use hkdf::Hkdf;
use limina_core::{Result, internal};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::config::AuthConfigError;

const SEAL_VERSION: u8 = 0x01;
const SALT: &[u8] = b"limina-workos";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// Accepts sealed values with or without base64 padding, like Python's decoder did.
const URL_SAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// AES-256-GCM sealing for cookie payloads, keyed from the configured cookie password.
pub struct SessionSealer {
    cipher: Aes256Gcm,
}

impl SessionSealer {
    pub fn new(password: &str, purpose: &str) -> std::result::Result<Self, AuthConfigError> {
        let mut key = [0_u8; 32];
        Hkdf::<Sha256>::new(Some(SALT), password.as_bytes())
            .expand(purpose.as_bytes(), &mut key)
            .map_err(|_| AuthConfigError::new("Cannot derive the cookie key."))?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| AuthConfigError::new("Cannot derive the cookie key."))?;
        Ok(Self { cipher })
    }

    pub fn seal(&self, payload: &Value) -> Result<String> {
        let plaintext = serde_json::to_vec(payload).map_err(internal)?;
        let nonce_bytes: [u8; NONCE_LEN] = random_bytes()?;
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, Payload { msg: &plaintext, aad: &[SEAL_VERSION] })
            .map_err(|_| internal("cannot seal the cookie"))?;
        let mut sealed = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        sealed.push(SEAL_VERSION);
        sealed.extend_from_slice(&nonce_bytes);
        sealed.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(sealed))
    }

    /// The payload, or `None` for anything forged, tampered with, sealed with another password or
    /// purpose, or not a JSON object.
    pub fn unseal(&self, value: &str) -> Option<Map<String, Value>> {
        let raw = URL_SAFE_LENIENT.decode(value).ok()?;
        if raw.len() < 1 + NONCE_LEN + TAG_LEN || raw[0] != SEAL_VERSION {
            return None;
        }
        let nonce_bytes: [u8; NONCE_LEN] = raw[1..=NONCE_LEN].try_into().ok()?;
        let plaintext = self
            .cipher
            .decrypt(
                &Nonce::from(nonce_bytes),
                Payload { msg: &raw[1 + NONCE_LEN..], aad: &[SEAL_VERSION] },
            )
            .ok()?;
        match serde_json::from_slice(&plaintext).ok()? {
            Value::Object(payload) => Some(payload),
            _ => None,
        }
    }
}

/// A PKCE `(verifier, S256 challenge)` pair.
pub fn pkce_pair() -> Result<(String, String)> {
    let verifier = token_urlsafe(48)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok((verifier, challenge))
}

/// `nbytes` random bytes as unpadded base64url (Python's `secrets.token_urlsafe`).
pub fn token_urlsafe(nbytes: usize) -> Result<String> {
    let mut bytes = vec![0_u8; nbytes];
    getrandom::fill(&mut bytes).map_err(|error| internal(format!("no OS randomness: {error}")))?;
    Ok(general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|error| internal(format!("no OS randomness: {error}")))?;
    Ok(bytes)
}

/// Lowercase hex SHA-256, used to key per-refresh-token state without keeping the token.
pub(crate) fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}
