use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use rand::Rng;
use sha3::{Sha3_256, Digest};
use std::fs;

pub type SessionKey = [u8; 32];

pub fn generate_session_key() -> SessionKey {
    let mut key = SessionKey::default();
    rand::thread_rng().fill(&mut key);
    key
}

pub fn envelope_encrypt(plaintext: &[u8], key: &SessionKey) -> Result<(Vec<u8>, [u8; 12]), aes_gcm::Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes: [u8; 12] = rand::thread_rng().gen();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|_| aes_gcm::Error)?;
    Ok((ciphertext, nonce_bytes))
}

pub fn decrypt(ciphertext: &[u8], key: &SessionKey, nonce: &[u8; 12]) -> Result<Vec<u8>, aes_gcm::Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext).map_err(|_| aes_gcm::Error)
}

pub async fn fetch_from_magalu(uri: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Placeholder: implement S3-compatible client for Magalu Object Storage
    Ok(fs::read(uri)?)
}

pub async fn upload_to_magalu(uri: &str, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    // Placeholder: implement S3-compatible client for Magalu Object Storage
    fs::write(uri, data)?;
    Ok(())
}

pub fn compute_seal(job_name: &str, model_uri: &str, residence: u64) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(job_name);
    hasher.update(model_uri);
    hasher.update(&residence.to_le_bytes());
    hex::encode(hasher.finalize())
}