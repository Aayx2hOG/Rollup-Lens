use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const HASH_BYTES: usize = 32;

type Hash = [u8; HASH_BYTES];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf_index: usize,
    pub leaf_count: usize,
    pub leaf_hash: String,
    pub root: String,
    pub path: Vec<ProofStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofStep {
    pub sibling_hash: String,
    pub sibling_side: SiblingSide,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SiblingSide {
    Left,
    Right,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MerkleError {
    #[error("cannot create a proof for an empty tree")]
    EmptyTree,
    #[error("leaf index {index} is out of range for {leaf_count} leaves")]
    IndexOutOfRange { index: usize, leaf_count: usize },
    #[error("invalid hex hash")]
    InvalidHex,
    #[error("invalid proof: {0}")]
    InvalidProof(String),
}

pub fn root_hex(leaves: &[Value]) -> String {
    encode_hex(&root_hash(leaves))
}

pub fn root_hash(leaves: &[Value]) -> Hash {
    if leaves.is_empty() {
        return empty_hash();
    }

    let mut level: Vec<Hash> = leaves.iter().map(hash_json_leaf).collect();
    while level.len() > 1 {
        level = next_level(&level);
    }

    level[0]
}

pub fn create_proof(leaves: &[Value], index: usize) -> Result<MerkleProof, MerkleError> {
    if leaves.is_empty() {
        return Err(MerkleError::EmptyTree);
    }
    if index >= leaves.len() {
        return Err(MerkleError::IndexOutOfRange {
            index,
            leaf_count: leaves.len(),
        });
    }

    let mut path = Vec::new();
    let mut current_index = index;
    let mut level: Vec<Hash> = leaves.iter().map(hash_json_leaf).collect();

    while level.len() > 1 {
        let is_right_child = current_index % 2 == 1;
        let sibling_index = if is_right_child {
            current_index - 1
        } else {
            (current_index + 1).min(level.len() - 1)
        };

        path.push(ProofStep {
            sibling_hash: encode_hex(&level[sibling_index]),
            sibling_side: if is_right_child {
                SiblingSide::Left
            } else {
                SiblingSide::Right
            },
        });

        current_index /= 2;
        level = next_level(&level);
    }

    let leaf_hash = hash_json_leaf(&leaves[index]);
    let root = level[0];

    Ok(MerkleProof {
        leaf_index: index,
        leaf_count: leaves.len(),
        leaf_hash: encode_hex(&leaf_hash),
        root: encode_hex(&root),
        path,
    })
}

pub fn verify_proof(
    leaf: &Value,
    proof: &MerkleProof,
    expected_root: &str,
) -> Result<bool, MerkleError> {
    if proof.leaf_count == 0 {
        return Err(MerkleError::InvalidProof(
            "leaf_count must be greater than zero".to_owned(),
        ));
    }
    if proof.leaf_index >= proof.leaf_count {
        return Err(MerkleError::InvalidProof(
            "leaf_index must be smaller than leaf_count".to_owned(),
        ));
    }

    let mut hash = hash_json_leaf(leaf);
    if encode_hex(&hash) != proof.leaf_hash {
        return Ok(false);
    }

    for step in &proof.path {
        let sibling = decode_hex_hash(&step.sibling_hash)?;
        hash = match step.sibling_side {
            SiblingSide::Left => hash_node(&sibling, &hash),
            SiblingSide::Right => hash_node(&hash, &sibling),
        };
    }

    Ok(encode_hex(&hash) == normalize_hex(expected_root)?
        && proof.root == normalize_hex(expected_root)?)
}

pub fn hash_json_leaf(value: &Value) -> Hash {
    hash_leaf(&canonical_json_bytes(value))
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    write_canonical_json(value, &mut output);
    output
}

fn next_level(level: &[Hash]) -> Vec<Hash> {
    level
        .chunks(2)
        .map(|chunk| {
            let left = chunk[0];
            let right = *chunk.get(1).unwrap_or(&left);
            hash_node(&left, &right)
        })
        .collect()
}

fn empty_hash() -> Hash {
    Sha256::digest(b"zk-rollup-merkle-empty-v1").into()
}

fn hash_leaf(bytes: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    hasher.finalize().into()
}

fn hash_node(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update([0x01]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn decode_hex_hash(input: &str) -> Result<Hash, MerkleError> {
    let normalized = normalize_hex(input)?;
    let mut bytes = [0u8; HASH_BYTES];
    for (index, chunk) in normalized.as_bytes().chunks(2).enumerate() {
        bytes[index] = (hex_value(chunk[0])? << 4) | hex_value(chunk[1])?;
    }
    Ok(bytes)
}

fn normalize_hex(input: &str) -> Result<String, MerkleError> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex.len() != HASH_BYTES * 2 {
        return Err(MerkleError::InvalidHex);
    }
    if !hex.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MerkleError::InvalidHex);
    }
    Ok(hex.to_ascii_lowercase())
}

fn hex_value(byte: u8) -> Result<u8, MerkleError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(MerkleError::InvalidHex),
    }
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Value::String(value) => {
            serde_json::to_writer(output, value)
                .expect("serializing a JSON string should not fail");
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output);
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)
                    .expect("serializing a JSON object key should not fail");
                output.push(b':');
                write_canonical_json(value, output);
            }
            output.push(b'}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn root_generation_is_deterministic() {
        let leaves = json_leaves(r#"[{"b":2,"a":1},{"id":2},{"id":3}]"#);

        assert_eq!(root_hex(&leaves), root_hex(&leaves));
    }

    #[test]
    fn object_key_order_does_not_change_leaf_hash() {
        let left = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        let right = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();

        assert_eq!(hash_json_leaf(&left), hash_json_leaf(&right));
    }

    #[test]
    fn empty_tree_has_stable_root_and_cannot_create_proof() {
        assert_eq!(
            root_hex(&[]),
            "67cdfd540b12369175fb990d77f15da3497008e14aba58f034948426d84c2b96"
        );
        assert_eq!(create_proof(&[], 0), Err(MerkleError::EmptyTree));
    }

    #[test]
    fn one_leaf_proof_verifies() {
        let leaves = vec![json!({"account":"alice","balance":100})];
        let proof = create_proof(&leaves, 0).unwrap();

        assert!(verify_proof(&leaves[0], &proof, &proof.root).unwrap());
        assert!(proof.path.is_empty());
    }

    #[test]
    fn odd_leaf_tree_proofs_verify_for_every_leaf() {
        let leaves = json_leaves(r#"[1,2,3,4,5]"#);

        for index in 0..leaves.len() {
            let proof = create_proof(&leaves, index).unwrap();
            assert!(verify_proof(&leaves[index], &proof, &root_hex(&leaves)).unwrap());
        }
    }

    #[test]
    fn large_tree_proofs_verify_for_every_leaf() {
        let leaves: Vec<Value> = (0..257)
            .map(|index| json!({"account": format!("user-{index}"), "balance": index * 10}))
            .collect();

        for index in 0..leaves.len() {
            let proof = create_proof(&leaves, index).unwrap();
            assert!(verify_proof(&leaves[index], &proof, &root_hex(&leaves)).unwrap());
        }
    }

    #[test]
    fn tampered_leaf_fails_verification() {
        let leaves = json_leaves(r#"[{"id":1},{"id":2},{"id":3}]"#);
        let proof = create_proof(&leaves, 1).unwrap();

        assert!(!verify_proof(&json!({"id":200}), &proof, &proof.root).unwrap());
    }

    #[test]
    fn tampered_proof_fails_verification() {
        let leaves = json_leaves(r#"[{"id":1},{"id":2},{"id":3}]"#);
        let mut proof = create_proof(&leaves, 1).unwrap();
        proof.path[0].sibling_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();

        assert!(!verify_proof(&leaves[1], &proof, &root_hex(&leaves)).unwrap());
    }

    #[test]
    fn out_of_range_proof_is_rejected() {
        let leaves = json_leaves(r#"[1,2]"#);

        assert_eq!(
            create_proof(&leaves, 2),
            Err(MerkleError::IndexOutOfRange {
                index: 2,
                leaf_count: 2
            })
        );
    }

    proptest! {
        #[test]
        fn generated_proofs_verify(values in prop::collection::vec(any::<u64>(), 1..80)) {
            let leaves: Vec<Value> = values.into_iter().map(Value::from).collect();

            for index in 0..leaves.len() {
                let proof = create_proof(&leaves, index).unwrap();
                prop_assert!(verify_proof(&leaves[index], &proof, &root_hex(&leaves)).unwrap());
            }
        }
    }

    fn json_leaves(input: &str) -> Vec<Value> {
        match serde_json::from_str(input).unwrap() {
            Value::Array(values) => values,
            _ => panic!("test input must be a JSON array"),
        }
    }
}
