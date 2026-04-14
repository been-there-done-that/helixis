use std::collections::BTreeMap;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use application::ports::repositories::{RepositoryError, SecretRepository};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const NONCE_LEN: usize = 12;

pub struct PostgresSecretRepository {
    pool: PgPool,
    cipher: SecretCipher,
}

impl PostgresSecretRepository {
    pub fn new(pool: PgPool, secret_key: &str) -> Self {
        Self {
            pool,
            cipher: SecretCipher::new(secret_key),
        }
    }
}

#[derive(Clone)]
struct SecretCipher {
    cipher: Aes256Gcm,
}

impl SecretCipher {
    fn new(secret_key: &str) -> Self {
        let key_material = Sha256::digest(secret_key.as_bytes());
        let cipher =
            Aes256Gcm::new_from_slice(&key_material).expect("32-byte secret key derivation failed");
        Self { cipher }
    }

    fn encrypt(&self, plaintext: &str) -> Result<String, RepositoryError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let mut encoded = nonce_bytes.to_vec();
        encoded.extend(ciphertext);
        Ok(STANDARD.encode(encoded))
    }

    fn decrypt(&self, ciphertext: &str) -> Result<String, RepositoryError> {
        let decoded = STANDARD
            .decode(ciphertext)
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        if decoded.len() <= NONCE_LEN {
            return Err(RepositoryError::DatabaseError(
                "stored secret payload is invalid".to_string(),
            ));
        }

        let (nonce, encrypted) = decoded.split_at(NONCE_LEN);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), encrypted)
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        String::from_utf8(plaintext).map_err(|err| RepositoryError::DatabaseError(err.to_string()))
    }
}

#[async_trait]
impl SecretRepository for PostgresSecretRepository {
    async fn put_secret(
        &self,
        tenant_id: Uuid,
        key: &str,
        value: &str,
    ) -> Result<(), RepositoryError> {
        let encrypted_value = self.cipher.encrypt(value)?;

        sqlx::query(
            r#"
            INSERT INTO secrets (tenant_id, secret_key, encrypted_value, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (tenant_id, secret_key)
            DO UPDATE
            SET encrypted_value = EXCLUDED.encrypted_value,
                updated_at = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(key)
        .bind(encrypted_value)
        .execute(&self.pool)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(())
    }

    async fn get_tenant_secrets(
        &self,
        tenant_id: Uuid,
    ) -> Result<BTreeMap<String, String>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT secret_key, encrypted_value
            FROM secrets
            WHERE tenant_id = $1
            ORDER BY secret_key ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let mut secrets = BTreeMap::new();
        for row in rows {
            let key: String = row.get("secret_key");
            let encrypted_value: String = row.get("encrypted_value");
            let value = self.cipher.decrypt(&encrypted_value)?;
            secrets.insert(key, value);
        }

        Ok(secrets)
    }
}
