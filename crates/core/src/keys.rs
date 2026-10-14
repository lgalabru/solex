//! Solana CLI compatible keypair files: a JSON array of 64 bytes
//! (`secret(32) || public(32)`).

use std::path::Path;

use crate::{CoreError, Result};

pub struct Keypair {
    bytes: [u8; 64],
}

// Never print the secret half, even in debug output.
impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keypair")
            .field("pubkey", &self.pubkey())
            .finish()
    }
}

impl Keypair {
    pub fn generate() -> Self {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::thread_rng());
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(signing.as_bytes());
        bytes[32..].copy_from_slice(signing.verifying_key().as_bytes());
        Self { bytes }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| CoreError::io("read", path, e))?;
        let vec: Vec<u8> = serde_json::from_str(&json).map_err(|e| CoreError::Keypair {
            path: path.to_path_buf(),
            reason: format!("not a JSON byte array ({e})"),
        })?;
        let bytes: [u8; 64] = vec.as_slice().try_into().map_err(|_| CoreError::Keypair {
            path: path.to_path_buf(),
            reason: format!("expected 64 bytes, found {}", vec.len()),
        })?;
        Ok(Self { bytes })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::io("create", parent, e))?;
        }
        let json = serde_json::to_string(&self.bytes.to_vec())
            .map_err(|e| CoreError::message(format!("failed to encode keypair: {e}")))?;
        std::fs::write(path, json).map_err(|e| CoreError::io("write", path, e))
    }

    /// Base58 public key.
    pub fn pubkey(&self) -> String {
        bs58::encode(&self.bytes[32..]).into_string()
    }
}

/// Read only the public key of a keypair file.
pub fn pubkey_of(path: &Path) -> Result<String> {
    Ok(Keypair::read(path)?.pubkey())
}

/// Validate a base58 program id (32 bytes).
pub fn validate_pubkey(s: &str) -> std::result::Result<(), String> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| format!("not base58 ({e})"))?;
    if bytes.len() != 32 {
        return Err(format!("decodes to {} bytes, expected 32", bytes.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_pubkey() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.json");
        let kp = Keypair::generate();
        kp.write(&path).unwrap();
        assert_eq!(pubkey_of(&path).unwrap(), kp.pubkey());
        assert!(validate_pubkey(&kp.pubkey()).is_ok());
    }

    #[test]
    fn rejects_wrong_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.json");
        std::fs::write(&path, serde_json::to_string(&vec![7u8; 63]).unwrap()).unwrap();
        let err = Keypair::read(&path).unwrap_err();
        assert!(
            err.to_string().contains("expected 64 bytes, found 63"),
            "{err}"
        );
    }
}
