use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// AES-256-GCM encryptor.
///
/// Wire format is byte-compatible with the legacy Go gateway:
/// key = SHA256(secret); ciphertext = base64(nonce || gcm_seal(nonce, plaintext)).
/// Existing accounts.json files therefore remain readable after the rewrite.
pub struct Encryptor {
    cipher: Aes256Gcm,
}

impl Encryptor {
    pub fn new(secret: &str) -> Self {
        let digest = Sha256::digest(secret.as_bytes());
        let cipher = Aes256Gcm::new_from_slice(&digest).expect("sha256 key must be 32 bytes");
        Self { cipher }
    }

    pub fn encrypt(&self, plaintext: &str) -> anyhow::Result<String> {
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("encrypt failed: {}", e))?;
        let mut combined = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);
        Ok(B64.encode(combined))
    }

    pub fn decrypt(&self, encoded: &str) -> anyhow::Result<String> {
        let data = B64.decode(encoded)?;
        if data.len() < 12 {
            anyhow::bail!("ciphertext too short");
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("decrypt failed: {}", e))?;
        Ok(String::from_utf8(plaintext)?)
    }
}
