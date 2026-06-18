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

Generate a batch commitment from the local chain:

```bash
cargo run -p chain-cli -- --chain-file chain.json rollup batch \
  --batch-number 0 \
  --start-height 1 \
  --end-height 1
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

Hex strings from the CLI should be submitted as Solidity `bytes32` values with a
`0x` prefix.

## Next Hardening Steps

- Replace owner-based `rejectChallengedBatch` with an on-chain fraud-proof or
  validity-proof verifier.
- Add withdrawal proof verification against `finalizedStateRoot`.
- Store submitted L2 forced-withdrawal request IDs in batch data and verify they
  were included before allowing the sequencer to mark them resolved.
- Add a Foundry or Hardhat test harness before deploying beyond local testing.
