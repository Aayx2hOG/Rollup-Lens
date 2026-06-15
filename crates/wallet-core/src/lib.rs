use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevKeypair {
    pub secret_key_base64: String,
    pub public_key_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedMessage {
    pub message: String,
    pub public_key_base64: String,
    pub signature_base64: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WalletError {
    #[error("invalid base64 data")]
    InvalidBase64,
    #[error("invalid secret key")]
    InvalidSecretKey,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("secret key does not match public key")]
    KeypairMismatch,
}

pub fn create_keypair() -> DevKeypair {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    DevKeypair {
        secret_key_base64: BASE64.encode(signing_key.to_bytes()),
        public_key_base64: BASE64.encode(verifying_key.to_bytes()),
    }
}

pub fn address_from_public_key(public_key_base64: &str) -> Result<String, WalletError> {
    let public_key = decode_public_key(public_key_base64)?;
    let digest = Sha256::digest(public_key.as_bytes());
    Ok(BASE64.encode(&digest[..20]))
}

pub fn public_key_from_secret(secret_key_base64: &str) -> Result<String, WalletError> {
    let signing_key = decode_signing_key(secret_key_base64)?;
    Ok(BASE64.encode(signing_key.verifying_key().to_bytes()))
}

pub fn keypair_from_secret(secret_key_base64: &str) -> Result<DevKeypair, WalletError> {
    Ok(DevKeypair {
        secret_key_base64: secret_key_base64.to_owned(),
        public_key_base64: public_key_from_secret(secret_key_base64)?,
    })
}

pub fn validate_keypair(keypair: &DevKeypair) -> Result<(), WalletError> {
    let derived_public_key = public_key_from_secret(&keypair.secret_key_base64)?;
    if derived_public_key == keypair.public_key_base64 {
        Ok(())
    } else {
        Err(WalletError::KeypairMismatch)
    }
}

pub fn sign_message(secret_key_base64: &str, message: &str) -> Result<SignedMessage, WalletError> {
    let signing_key = decode_signing_key(secret_key_base64)?;
    let signature = signing_key.sign(message.as_bytes());

    Ok(SignedMessage {
        message: message.to_owned(),
        public_key_base64: BASE64.encode(signing_key.verifying_key().to_bytes()),
        signature_base64: BASE64.encode(signature.to_bytes()),
    })
}

pub fn verify_message(
    public_key_base64: &str,
    message: &str,
    signature_base64: &str,
) -> Result<bool, WalletError> {
    let public_key = decode_public_key(public_key_base64)?;
    let signature = decode_signature(signature_base64)?;

    Ok(public_key.verify(message.as_bytes(), &signature).is_ok())
}

fn decode_signing_key(secret_key_base64: &str) -> Result<SigningKey, WalletError> {
    let bytes = BASE64
        .decode(secret_key_base64)
        .map_err(|_| WalletError::InvalidBase64)?;
    let secret: [u8; 32] = bytes
        .try_into()
        .map_err(|_| WalletError::InvalidSecretKey)?;

    Ok(SigningKey::from_bytes(&secret))
}

fn decode_public_key(public_key_base64: &str) -> Result<VerifyingKey, WalletError> {
    let bytes = BASE64
        .decode(public_key_base64)
        .map_err(|_| WalletError::InvalidBase64)?;
    let public: [u8; 32] = bytes
        .try_into()
        .map_err(|_| WalletError::InvalidPublicKey)?;

    VerifyingKey::from_bytes(&public).map_err(|_| WalletError::InvalidPublicKey)
}

fn decode_signature(signature_base64: &str) -> Result<Signature, WalletError> {
    let bytes = BASE64
        .decode(signature_base64)
        .map_err(|_| WalletError::InvalidBase64)?;
    let signature: [u8; 64] = bytes
        .try_into()
        .map_err(|_| WalletError::InvalidSignature)?;

    Ok(Signature::from_bytes(&signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_keypair_and_derives_address() {
        let keypair = create_keypair();
        let address = address_from_public_key(&keypair.public_key_base64).unwrap();

        assert_eq!(
            public_key_from_secret(&keypair.secret_key_base64).unwrap(),
            keypair.public_key_base64
        );
        assert!(!address.is_empty());
    }

    #[test]
    fn signs_and_verifies_message() {
        let keypair = create_keypair();
        let signed = sign_message(&keypair.secret_key_base64, "hello rollup").unwrap();

        let verified = verify_message(
            &signed.public_key_base64,
            &signed.message,
            &signed.signature_base64,
        )
        .unwrap();

        assert!(verified);
    }

    #[test]
    fn imports_and_validates_keypair_from_secret() {
        let keypair = create_keypair();
        let imported = keypair_from_secret(&keypair.secret_key_base64).unwrap();

        assert_eq!(imported, keypair);
        assert_eq!(validate_keypair(&imported), Ok(()));
    }

    #[test]
    fn rejects_mismatched_keypair() {
        let first = create_keypair();
        let second = create_keypair();
        let mismatched = DevKeypair {
            secret_key_base64: first.secret_key_base64,
            public_key_base64: second.public_key_base64,
        };

        assert_eq!(
            validate_keypair(&mismatched),
            Err(WalletError::KeypairMismatch)
        );
    }

    #[test]
    fn rejects_tampered_message() {
        let keypair = create_keypair();
        let signed = sign_message(&keypair.secret_key_base64, "hello rollup").unwrap();

        let verified = verify_message(
            &signed.public_key_base64,
            "hello attacker",
            &signed.signature_base64,
        )
        .unwrap();

        assert!(!verified);
    }

    #[test]
    fn rejects_invalid_signature_encoding() {
        let keypair = create_keypair();
        let error = verify_message(&keypair.public_key_base64, "hello", "not-base64").unwrap_err();

        assert_eq!(error, WalletError::InvalidBase64);
    }
}
