use anyhow::{Context, Result, anyhow};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use uuid::Uuid;

use crate::auth::ServerMode;

const DEVELOPMENT_KEY_HEX: &str =
    "a5c3f1458279dfb241239378dbefa6b8d2ab32703cba1768343712fd37ac1f04";

#[derive(Clone)]
pub struct SecretCipher {
    cipher: ChaCha20Poly1305,
}

impl SecretCipher {
    pub fn initialize(mode: ServerMode, configured_key_hex: Option<&str>) -> Result<Self> {
        let key_hex = match (mode, configured_key_hex) {
            (_, Some(value)) if !value.trim().is_empty() => value.trim(),
            (ServerMode::Development, _) => DEVELOPMENT_KEY_HEX,
            (ServerMode::Production, _) => {
                return Err(anyhow!(
                    "CRONY_SECRET_MASTER_KEY_HEX is required in production mode"
                ));
            }
        };
        let key = hex::decode(key_hex).context("decode CRONY_SECRET_MASTER_KEY_HEX")?;
        if key.len() != 32 {
            return Err(anyhow!(
                "CRONY_SECRET_MASTER_KEY_HEX must decode to exactly 32 bytes"
            ));
        }
        Ok(Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(&key)),
        })
    }

    pub fn encrypt(
        &self,
        corp_id: Uuid,
        secret_id: Uuid,
        name: &str,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let random = Uuid::new_v4();
        let mut nonce_bytes = [0_u8; 12];
        nonce_bytes.copy_from_slice(&random.as_bytes()[..12]);
        let aad = associated_data(corp_id, secret_id, name);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("encrypt secret"))?;
        Ok((ciphertext, nonce_bytes.to_vec()))
    }

    pub fn decrypt(
        &self,
        corp_id: Uuid,
        secret_id: Uuid,
        name: &str,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(anyhow!("stored secret nonce has an invalid length"));
        }
        let aad = associated_data(corp_id, secret_id, name);
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("decrypt secret"))
    }
}

fn associated_data(corp_id: Uuid, secret_id: Uuid, name: &str) -> String {
    format!("{corp_id}:{secret_id}:{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciphertext_is_bound_to_corp_secret_and_name() {
        let cipher =
            SecretCipher::initialize(ServerMode::Development, None).expect("development cipher");
        let corp_id = Uuid::new_v4();
        let secret_id = Uuid::new_v4();
        let (ciphertext, nonce) = cipher
            .encrypt(corp_id, secret_id, "deploy-token", b"super-secret")
            .expect("encrypt");
        assert_ne!(ciphertext, b"super-secret");
        assert_eq!(
            cipher
                .decrypt(corp_id, secret_id, "deploy-token", &ciphertext, &nonce)
                .expect("decrypt"),
            b"super-secret"
        );
        assert!(
            cipher
                .decrypt(
                    Uuid::new_v4(),
                    secret_id,
                    "deploy-token",
                    &ciphertext,
                    &nonce
                )
                .is_err()
        );
    }
}
