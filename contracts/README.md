# Rollup Settlement Contract

`RollupSettlement.sol` is an optimistic settlement layer for the Rust rollup
simulator. The Rust chain still owns local execution and state-root creation;
the contract owns finality rules for submitted rollup batches.

## Security Rules

- Only the configured sequencer can submit batches.
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

The API response includes both named `args` and `ordered_args`. The latter is
the direct argument order for `submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)`.

## Tests

The Foundry harness in `contracts/test/RollupSettlement.t.sol` covers:

- bonded sequencer submission
- challenge-window finalization
- rejection of challenged batches
- replacement of a rejected batch number

Run:

```bash
forge test
```

## Next Hardening Steps

- Replace owner-based `rejectChallengedBatch` with an on-chain fraud-proof or
  validity-proof verifier.
- Add withdrawal proof verification against `finalizedStateRoot`.
- Store submitted L2 forced-withdrawal request IDs in batch data and verify they
  were included before allowing the sequencer to mark them resolved.
- Add a Foundry or Hardhat test harness before deploying beyond local testing.
