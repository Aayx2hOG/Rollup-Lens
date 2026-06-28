use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const HASH_BYTES: usize = 32;
pub const SPARSE_TREE_DEPTH: usize = HASH_BYTES * 8;

pub type Hash = [u8; HASH_BYTES];

const SMT_KEY_DOMAIN: &[u8] = b"zk-rollup-smt-key-v1";
const SMT_EMPTY_LEAF_DOMAIN: &[u8] = b"zk-rollup-smt-empty-leaf-v1";
const SMT_OCCUPIED_LEAF_DOMAIN: &[u8] = b"zk-rollup-smt-occupied-leaf-v1";
const SMT_INTERNAL_DOMAIN: &[u8] = b"zk-rollup-smt-internal-v1";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SparseMerkleProof {
    pub key: String,
    pub value: Option<String>,
    pub siblings: Vec<String>,
    pub root: String,
}

#[derive(Debug, Clone)]
pub struct SparseMerkleTree {
    leaves: BTreeMap<Hash, Vec<u8>>,
    nodes: BTreeMap<(usize, Hash), Hash>,
    empty_hashes: Vec<Hash>,
    root: Hash,
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
    #[error("duplicate sparse Merkle key")]
    DuplicateSparseKey,
}

impl SparseMerkleTree {
    pub fn new<I, K, V>(entries: I) -> Result<Self, MerkleError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let empty_hashes = sparse_empty_hashes();
        let mut leaves = BTreeMap::new();
        let mut nodes = BTreeMap::new();

        for (key, value) in entries {
            let path = sparse_key_hash(key.as_ref());
            if leaves.insert(path, value.as_ref().to_vec()).is_some() {
                return Err(MerkleError::DuplicateSparseKey);
            }
            nodes.insert(
                (SPARSE_TREE_DEPTH, path),
                sparse_leaf_hash(&path, value.as_ref()),
            );
        }

        for depth in (0..SPARSE_TREE_DEPTH).rev() {
            let child_depth = depth + 1;
            let parents: BTreeSet<Hash> = nodes
                .keys()
                .filter(|(node_depth, _)| *node_depth == child_depth)
                .map(|(_, path)| prefix(*path, depth))
                .collect();

            for parent in parents {
                let left_path = child_path(parent, depth, false);
                let right_path = child_path(parent, depth, true);
                let left = nodes
                    .get(&(child_depth, left_path))
                    .copied()
                    .unwrap_or(empty_hashes[child_depth]);
                let right = nodes
                    .get(&(child_depth, right_path))
                    .copied()
                    .unwrap_or(empty_hashes[child_depth]);
                nodes.insert((depth, parent), sparse_internal_hash(&left, &right));
            }
        }

        let root = nodes
            .get(&(0, [0; HASH_BYTES]))
            .copied()
            .unwrap_or(empty_hashes[0]);
        Ok(Self {
            leaves,
            nodes,
            empty_hashes,
            root,
        })
    }

    pub fn root_hash(&self) -> Hash {
        self.root
    }

    pub fn root_hex(&self) -> String {
        encode_hex(&self.root)
    }

    pub fn prove(&self, key: &[u8]) -> SparseMerkleProof {
        let path = sparse_key_hash(key);
        let mut siblings = Vec::with_capacity(SPARSE_TREE_DEPTH);
        for child_depth in (1..=SPARSE_TREE_DEPTH).rev() {
            let parent_depth = child_depth - 1;
            let sibling = child_path(
                prefix(path, parent_depth),
                parent_depth,
                !bit_at(&path, parent_depth),
            );
            siblings.push(encode_hex(
                self.nodes
                    .get(&(child_depth, sibling))
                    .unwrap_or(&self.empty_hashes[child_depth]),
            ));
        }

        SparseMerkleProof {
            key: encode_hex(&path),
            value: self.leaves.get(&path).map(|value| encode_hex(value)),
            siblings,
            root: self.root_hex(),
        }
    }

    /// Inserts, replaces, or removes one value and recalculates only its
    /// leaf-to-root path.
    pub fn update(&mut self, key: &[u8], value: Option<&[u8]>) {
        let path = sparse_key_hash(key);
        match value {
            Some(value) => {
                self.leaves.insert(path, value.to_vec());
                self.nodes
                    .insert((SPARSE_TREE_DEPTH, path), sparse_leaf_hash(&path, value));
            }
            None => {
                self.leaves.remove(&path);
                self.nodes.remove(&(SPARSE_TREE_DEPTH, path));
            }
        }

        for depth in (0..SPARSE_TREE_DEPTH).rev() {
            let child_depth = depth + 1;
            let parent = prefix(path, depth);
            let left = self
                .nodes
                .get(&(child_depth, child_path(parent, depth, false)))
                .copied()
                .unwrap_or(self.empty_hashes[child_depth]);
            let right = self
                .nodes
                .get(&(child_depth, child_path(parent, depth, true)))
                .copied()
                .unwrap_or(self.empty_hashes[child_depth]);
            let hash = sparse_internal_hash(&left, &right);
            if hash == self.empty_hashes[depth] {
                self.nodes.remove(&(depth, parent));
            } else {
                self.nodes.insert((depth, parent), hash);
            }
        }
        self.root = self
            .nodes
            .get(&(0, [0; HASH_BYTES]))
            .copied()
            .unwrap_or(self.empty_hashes[0]);
    }
}

pub fn sparse_root_hex<I, K, V>(entries: I) -> Result<String, MerkleError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
{
    Ok(SparseMerkleTree::new(entries)?.root_hex())
}

pub fn verify_sparse_proof(
    key: &[u8],
    value: Option<&[u8]>,
    proof: &SparseMerkleProof,
    expected_root: &str,
) -> Result<bool, MerkleError> {
    if proof.siblings.len() != SPARSE_TREE_DEPTH {
        return Err(MerkleError::InvalidProof(format!(
            "sparse proof must contain {SPARSE_TREE_DEPTH} siblings"
        )));
    }

    let path = sparse_key_hash(key);
    if proof.key != encode_hex(&path)
        || proof.value != value.map(encode_hex)
        || proof.root != normalize_hex(expected_root)?
    {
        return Ok(false);
    }

    let mut hash = match value {
        Some(value) => sparse_leaf_hash(&path, value),
        None => sparse_empty_hashes()[SPARSE_TREE_DEPTH],
    };
    for (offset, sibling) in proof.siblings.iter().enumerate() {
        let depth = SPARSE_TREE_DEPTH - 1 - offset;
        let sibling = decode_hex_hash(sibling)?;
        hash = if bit_at(&path, depth) {
            sparse_internal_hash(&sibling, &hash)
        } else {
            sparse_internal_hash(&hash, &sibling)
        };
    }

    Ok(encode_hex(&hash) == normalize_hex(expected_root)?)
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

fn sparse_key_hash(key: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(SMT_KEY_DOMAIN);
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.finalize().into()
}

fn sparse_leaf_hash(key: &Hash, value: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(SMT_OCCUPIED_LEAF_DOMAIN);
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
    hasher.finalize().into()
}

fn sparse_internal_hash(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(SMT_INTERNAL_DOMAIN);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn sparse_empty_hashes() -> Vec<Hash> {
    let mut hashes = vec![[0; HASH_BYTES]; SPARSE_TREE_DEPTH + 1];
    hashes[SPARSE_TREE_DEPTH] = Sha256::digest(SMT_EMPTY_LEAF_DOMAIN).into();
    for depth in (0..SPARSE_TREE_DEPTH).rev() {
        hashes[depth] = sparse_internal_hash(&hashes[depth + 1], &hashes[depth + 1]);
    }
    hashes
}

fn bit_at(path: &Hash, depth: usize) -> bool {
    path[depth / 8] & (1 << (7 - depth % 8)) != 0
}

fn prefix(mut path: Hash, depth: usize) -> Hash {
    if depth == SPARSE_TREE_DEPTH {
        return path;
    }
    let byte = depth / 8;
    let bit = depth % 8;
    if bit == 0 {
        path[byte..].fill(0);
    } else {
        path[byte] &= 0xff << (8 - bit);
        path[byte + 1..].fill(0);
    }
    path
}

fn child_path(parent: Hash, parent_depth: usize, right: bool) -> Hash {
    let mut child = prefix(parent, parent_depth);
    if right {
        child[parent_depth / 8] |= 1 << (7 - parent_depth % 8);
    }
    prefix(child, parent_depth + 1)
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

    #[test]
    fn sparse_root_is_independent_of_insertion_order() {
        let first = SparseMerkleTree::new([
            (b"alice".as_slice(), b"100:0".as_slice()),
            (b"bob".as_slice(), b"50:2".as_slice()),
        ])
        .unwrap();
        let second = SparseMerkleTree::new([
            (b"bob".as_slice(), b"50:2".as_slice()),
            (b"alice".as_slice(), b"100:0".as_slice()),
        ])
        .unwrap();

        assert_eq!(first.root_hash(), second.root_hash());
    }

    #[test]
    fn sparse_inclusion_and_non_membership_proofs_verify() {
        let tree = SparseMerkleTree::new([
            (b"alice".as_slice(), b"100:0".as_slice()),
            (b"bob".as_slice(), b"50:2".as_slice()),
        ])
        .unwrap();
        let included = tree.prove(b"alice");
        let absent = tree.prove(b"carol");

        assert!(
            verify_sparse_proof(b"alice", Some(b"100:0"), &included, &tree.root_hex()).unwrap()
        );
        assert!(verify_sparse_proof(b"carol", None, &absent, &tree.root_hex()).unwrap());
        assert!(
            !verify_sparse_proof(b"alice", Some(b"999:0"), &included, &tree.root_hex()).unwrap()
        );
    }

    #[test]
    fn sparse_incremental_updates_match_rebuilding() {
        let mut tree = SparseMerkleTree::new([(b"alice".as_slice(), b"100:0".as_slice())]).unwrap();
        tree.update(b"bob", Some(b"50:2"));
        let rebuilt = SparseMerkleTree::new([
            (b"alice".as_slice(), b"100:0".as_slice()),
            (b"bob".as_slice(), b"50:2".as_slice()),
        ])
        .unwrap();
        assert_eq!(tree.root_hash(), rebuilt.root_hash());

        tree.update(b"alice", None);
        let rebuilt = SparseMerkleTree::new([(b"bob".as_slice(), b"50:2".as_slice())]).unwrap();
        assert_eq!(tree.root_hash(), rebuilt.root_hash());
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
