#[cfg(target_os = "zkvm")]
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevKeypair {
    pub secret_key_hex: String,
    pub address: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WalletError {
    #[error("invalid secret key")]
    InvalidSecretKey,
    #[error("invalid Ethereum address")]
    InvalidAddress,
    #[error("invalid Ethereum signature")]
    InvalidSignature,
    #[error("secret key does not match address")]
    KeypairMismatch,
}

pub fn create_keypair() -> DevKeypair {
    let signing_key = SigningKey::random(&mut OsRng);
    DevKeypair {
        secret_key_hex: format!("0x{}", hex::encode(signing_key.to_bytes())),
        address: address_from_verifying_key(signing_key.verifying_key()),
    }
}

pub fn address_from_secret(secret_key_hex: &str) -> Result<String, WalletError> {
    Ok(address_from_verifying_key(
        decode_signing_key(secret_key_hex)?.verifying_key(),
    ))
}

pub fn keypair_from_secret(secret_key_hex: &str) -> Result<DevKeypair, WalletError> {
    Ok(DevKeypair {
        secret_key_hex: normalize_hex(secret_key_hex),
        address: address_from_secret(secret_key_hex)?,
    })
}

pub fn validate_keypair(keypair: &DevKeypair) -> Result<(), WalletError> {
    if address_from_secret(&keypair.secret_key_hex)? == normalize_address(&keypair.address)? {
        Ok(())
    } else {
        Err(WalletError::KeypairMismatch)
    }
}

pub fn sign_digest(secret_key_hex: &str, digest: &[u8; 32]) -> Result<String, WalletError> {
    let (signature, recovery_id) = decode_signing_key(secret_key_hex)?
        .sign_prehash_recoverable(digest)
        .map_err(|_| WalletError::InvalidSignature)?;
    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&signature.to_bytes());
    bytes[64] = recovery_id.to_byte() + 27;
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn decode_signature(signature_hex: &str) -> Result<(Signature, RecoveryId), WalletError> {
    let bytes = decode_hex(signature_hex).map_err(|_| WalletError::InvalidSignature)?;
    if bytes.len() != 65 {
        return Err(WalletError::InvalidSignature);
    }
    let signature = Signature::try_from(&bytes[..64]).map_err(|_| WalletError::InvalidSignature)?;
    let recovery_byte = match bytes[64] {
        0 | 1 => bytes[64],
        27 | 28 => bytes[64] - 27,
        _ => return Err(WalletError::InvalidSignature),
    };
    let recovery_id =
        RecoveryId::try_from(recovery_byte).map_err(|_| WalletError::InvalidSignature)?;
    Ok((signature, recovery_id))
}

#[cfg(not(target_os = "zkvm"))]
pub fn recover_address(digest: &[u8; 32], signature_hex: &str) -> Result<String, WalletError> {
    let (signature, recovery_id) = decode_signature(signature_hex)?;
    let verifying_key = VerifyingKey::recover_from_prehash(digest, &signature, recovery_id)
        .map_err(|_| WalletError::InvalidSignature)?;
    Ok(address_from_verifying_key(&verifying_key))
}

#[cfg(target_os = "zkvm")]
pub fn recover_address(digest: &[u8; 32], signature_hex: &str) -> Result<String, WalletError> {
    let (signature, recovery_id) = decode_signature(signature_hex)?;
    sp1_lib::unconstrained!({
        let verifying_key = VerifyingKey::recover_from_prehash(digest, &signature, recovery_id)
            .expect("valid Ethereum signature");
        sp1_zkvm::io::hint_slice(verifying_key.to_encoded_point(true).as_bytes());
    });
    let encoded_key = sp1_zkvm::io::read_vec();
    let verifying_key =
        VerifyingKey::from_sec1_bytes(&encoded_key).map_err(|_| WalletError::InvalidSignature)?;
    verifying_key
        .verify_prehash(digest, &signature)
        .map_err(|_| WalletError::InvalidSignature)?;
    Ok(address_from_verifying_key(&verifying_key))
}

pub fn normalize_address(address: &str) -> Result<String, WalletError> {
    let bytes = decode_hex(address).map_err(|_| WalletError::InvalidAddress)?;
    if bytes.len() != 20 {
        return Err(WalletError::InvalidAddress);
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn address_from_verifying_key(verifying_key: &VerifyingKey) -> String {
    let encoded = verifying_key.to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn decode_signing_key(secret_key_hex: &str) -> Result<SigningKey, WalletError> {
    let bytes = decode_hex(secret_key_hex).map_err(|_| WalletError::InvalidSecretKey)?;
    SigningKey::from_slice(&bytes).map_err(|_| WalletError::InvalidSecretKey)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(value.strip_prefix("0x").unwrap_or(value))
}

fn normalize_hex(value: &str) -> String {
    format!(
        "0x{}",
        value
            .strip_prefix("0x")
            .unwrap_or(value)
            .to_ascii_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_ethereum_keypair_and_derives_address() {
        let keypair = create_keypair();
        assert_eq!(
            address_from_secret(&keypair.secret_key_hex).unwrap(),
            keypair.address
        );
        assert_eq!(keypair.address.len(), 42);
    }

    #[test]
    fn signs_digest_and_recovers_address() {
        let keypair = create_keypair();
        let digest: [u8; 32] = Keccak256::digest(b"hello rollup").into();
        let signature = sign_digest(&keypair.secret_key_hex, &digest).unwrap();
        assert_eq!(
            recover_address(&digest, &signature).unwrap(),
            keypair.address
        );
    }

    #[test]
    fn imports_and_validates_keypair() {
        let keypair = create_keypair();
        let imported = keypair_from_secret(&keypair.secret_key_hex).unwrap();
        assert_eq!(imported, keypair);
        assert_eq!(validate_keypair(&imported), Ok(()));
    }

    #[test]
    fn rejects_mismatched_keypair() {
        let first = create_keypair();
        let second = create_keypair();
        assert_eq!(
            validate_keypair(&DevKeypair {
                secret_key_hex: first.secret_key_hex,
                address: second.address,
            }),
            Err(WalletError::KeypairMismatch)
        );
    }

    #[test]
    fn rejects_invalid_signature() {
        assert_eq!(
            recover_address(&[0; 32], "0xdeadbeef"),
            Err(WalletError::InvalidSignature)
        );
    }
}
