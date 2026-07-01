# Rollup Settlement Contract

`RollupSettlement.sol` supports both optimistic and SP1 validity-proof
settlement. The Rust chain owns execution and state-root creation; the contract
owns proof verification and finality.

For new validity-rollup deployments, prefer
`ValidityRollupSettlement.sol`. It deliberately removes the optimistic
challenge, bond, pending-batch, withdrawal, and governance-resolution state.
`RollupSettlement.sol` remains available as the backwards-compatible hybrid.

## Validity-Only Storage

The minimal contract persists only:

- `finalizedStateRoot`
- `nextBatchNumber`
- one `batchCommitments[batchNumber]` value per batch

The batch commitment is `keccak256(publicValues)`, covering the batch number,
five roots/commitments, and transaction count proved by SP1. Full historical
fields are emitted in `BatchFinalized` for indexers instead of occupying
permanent storage slots.

The validity-only constructor is:

```solidity
new ValidityRollupSettlement(
    genesisStateRoot,
    officialSp1Verifier,
    programVKey
);
```

It exposes the same `submitBatchWithProof` signature already used by
`settlement-client --proof`, so the client requires only the new deployed
contract address.

## Security Rules

- Only the configured sequencer can submit batches.
- The validity path accepts only proofs for the immutable SP1 program key.
- Every validity proof is bound to the submitted batch number and commitments.
- Proven batches finalize immediately without a challenge window.
- The sequencer must keep a minimum ETH bond.
- Every batch must link from the latest finalized state root.
- Only one unfinalized batch can be pending at a time.
- Every batch commits to transaction, block-hash, and full batch data roots.
- Batches finalize only after the challenge window closes.
- Challenges require a bond and non-empty proof data to reduce spam.
- Challenged batches cannot finalize until the challenge is rejected or
  dismissed by governance or future verifier logic.
- Rejected batches slash sequencer bond, pay the challenger, and reopen the
  rejected batch number for replacement.
- Dismissed challenges forfeit the challenge bond to the sequencer bond and
  allow the batch to continue toward finality.
- Users can create forced-withdrawal requests if the sequencer censors exits.

## Contract Inputs From Rust

Generate a batch through the explorer API sequencer, then export the settlement
arguments:

```bash
curl http://127.0.0.1:3000/rollup/batches/0/settlement-calldata
```

The output maps to `submitBatch`:

```text
batch_number     -> batchNumber
prev_state_root  -> prevStateRoot
new_state_root   -> newStateRoot
transaction_root -> transactionRoot
block_hash_root  -> blockHashRoot
data_hash        -> dataHash
```

For validity settlement, generate `batch-proof.json` with `validity-prover`,
then submit it through:

```bash
cargo run -p settlement-client -- submit-batch \
  --batch-number 0 \
  --proof batch-proof.json
```

Without `--proof`, the client uses the existing optimistic `submitBatch` path.
With `--proof`, it calls `submitBatchWithProof`.

## Deployment

Deploy the official SP1 Groth16 verifier matching the exact SP1 SDK/circuit
version used by the prover. Pass its address and the artifact's `program_vkey`
to the `RollupSettlement` constructor. Both values are immutable because
silently changing either would change what computation the settlement contract
trusts.

An SDK upgrade requires generating a new proof, deploying or selecting its
matching official verifier, and deploying a new settlement contract (or an
explicitly governed upgrade design).

The API response includes both named `args` and `ordered_args`. The latter is
the direct argument order for `submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)`.

## Tests

The Foundry harness in `contracts/test/RollupSettlement.t.sol` covers:

- bonded sequencer submission
- challenge-window finalization
- rejection of challenged batches
- replacement of a rejected batch number
- immediate finality for a validity-proven batch
- rejection of invalid proofs
- rejection when proof public values differ from batch arguments

`contracts/test/ValidityRollupSettlement.t.sol` additionally covers minimal
commitment storage, sequential state updates, argument binding, and atomic
rejection without state writes.

Run:

```bash
forge test
```

## Next Hardening Steps

- Use the validity path as the production submission path and retain the
  optimistic path only if a deliberate fallback policy is required.
- Add an integration test against the real version-matched SP1 verifier, not
  only the deterministic verifier mock used by unit tests.
- Add withdrawal proof verification against `finalizedStateRoot`.
- Store submitted L2 forced-withdrawal request IDs in batch data and verify they
  were included before allowing the sequencer to mark them resolved.
- Add a Foundry or Hardhat test harness before deploying beyond local testing.
