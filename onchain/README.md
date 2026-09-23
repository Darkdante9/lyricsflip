# LyricsFlip on-chain contracts (Soroban)

Two Soroban smart contracts, ported from the project's earlier Cairo/Starknet
contracts:

- `contracts/lyricsflip` — game logic (rounds, cards, wagering scaffolding, answers)
- `contracts/lyricsflip-nft` — minter-gated NFT rewards

## Prerequisites

- Rust (version pinned in `../.tool-versions`)
- The `wasm32v1-none` target — **not** `wasm32-unknown-unknown`. Recent
  soroban-sdk releases require it on Rust 1.84+:

  ```bash
  rustup target add wasm32v1-none
  ```
- [Stellar CLI](https://developers.stellar.org/docs/tools/stellar-cli) (`stellar`) for deploying and invoking contracts, if you don't already have it.

## Round scoring and finalization

Each player's round score is the number of correct answers they submitted in that round. The contract also tracks the total time spent answering in that round so tie-breaks can be resolved fairly. `get_round_scores(round_id)` returns the final per-player score map for the round.

A round can be finalized when either:

- every required player/card answer in the round has been submitted, or
- the round deadline has passed (`round.end_time`)

`finalize_round(round_id)` is callable by any round participant and rejects early or duplicate finalization attempts. Once a round has been finalized, it cannot be finalized again; the second call fails and does not change `rounds_won` or emit another `RoundCompleted` event.

Winner selection follows the LF-005 rule:

- highest correct-answer count wins
- when several players are tied on score, the lowest total answer time wins
- if the score and total answer time are both tied, the tied players are co-winners
- if every player has a score of `0`, there are no winners

`PlayerStats.rounds_won` increments by exactly `1` for every winner when a round is finalized; non-winners and zero-score rounds do not change it. `RoundCompleted { round_id, winners, scores }` emits the finalized round id, the winning addresses, and the final score map for every player in the round.

## Build & test

```bash
cargo test                                    # unit tests, native target
cargo build --target wasm32v1-none --release  # produces deployable .wasm files
```

WASM output:

```
target/wasm32v1-none/release/lyricsflip.wasm
target/wasm32v1-none/release/lyricsflip_nft.wasm
```

## Deploying (testnet example)

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/lyricsflip.wasm \
  --source <YOUR_IDENTITY> \
  --network testnet \
  -- --owner <OWNER_ADDRESS>

stellar contract deploy \
  --wasm target/wasm32v1-none/release/lyricsflip_nft.wasm \
  --source <YOUR_IDENTITY> \
  --network testnet \
  -- --owner <OWNER_ADDRESS> --minter <LYRICSFLIP_CONTRACT_ID> \
     --token_name "LyricsFlip" --token_symbol "LFLIP" --base_uri "https://..."
```

Put the resulting contract IDs into the frontend's
`NEXT_PUBLIC_LYRICSFLIP_CONTRACT_ID` / `NEXT_PUBLIC_LYRICSFLIP_NFT_CONTRACT_ID`
environment variables (see `frontend/src/lib/stellar/stellarConfig.ts`).
