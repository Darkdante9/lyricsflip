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

## Deploy and seed (automated)

Two helper scripts live in `scripts/` and cover the full set-up flow for a
fresh testnet deployment.

### Prerequisites

1. Stellar CLI installed (`stellar`).
2. A funded identity called `me` on the target network:

   ```bash
   stellar keys generate --global me --network testnet --fund
   ```

3. Rust with the `wasm32v1-none` target (see above).
4. `jq` installed (required by `seed-cards.sh`).

### `scripts/deploy.sh`

Builds both WASMs, deploys them, wires the NFT minter to the game contract,
sets `cards_per_round`, and writes the two contract IDs into
`frontend/.env.local`.

```bash
cd onchain
./scripts/deploy.sh testnet   # or mainnet / futurenet
```

### `scripts/seed-cards.sh`

Reads `seed/cards.json` (≥ 5 cards per genre, using original and
public-domain lyric snippets) and calls `add_card` for every entry.  Run
after `deploy.sh` so the contract IDs are already in `frontend/.env.local`.

```bash
cd onchain
./scripts/seed-cards.sh testnet
```

### Manual deployment (alternative)

If you prefer to deploy by hand:

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

## Views for lobbies and catalogue size

| Function | Returns |
| --- | --- |
| `get_cards_count()` | Total number of cards added (`u64`) |
| `get_round_count()` | Total number of rounds created (`u64`) |
| `get_genre_card_count(genre)` | Number of cards in `genre` (`u32`) |
| `get_rounds(start, limit)` | Up to `limit` rounds from round id `start` (ids begin at 1), ascending |
| `get_open_rounds(start, limit)` | Ids of created-but-not-started rounds, oldest first; `start` is an offset |

Both paginated views clamp `limit` to `MAX_PAGE_LIMIT` (50) and return an
empty list once `start` is past the end.

## Error codes

Contract errors reach clients as `Error(Contract, #<code>)`, and the frontend
maps on the numeric code (`frontend/src/lib/stellar/errors.ts`). **Codes are
stable: never renumber or reuse one.** New variants take the next free number
and must be added here, in `errors.ts`, and in the `error_codes_are_stable`
test of the contract, which fails CI on any renumbering.

Every variant is currently referenced by the contract. The ones marked
*defensive* guard states that can't happen in practice.

### `lyricsflip`

| Code | Name | Meaning |
| --- | --- | --- |
| 1 | `AlreadyInitialized` | Constructor ran on an already-initialized contract (*defensive*) |
| 2 | `NonExistingRound` | No round with the given id |
| 3 | `RoundAlreadyStarted` | Tried to join a round that has already started |
| 4 | `NonExistingGenre` | `create_round` was called without a genre |
| 5 | `RoundAlreadyJoined` | Caller is already a player in the round |
| 6 | `InvalidCardsPerRound` | `set_cards_per_round` was called with 0 |
| 7 | `ArtistCardsIsZero` | No cards exist for the requested artist |
| 8 | `EmptyYearCards` | No cards exist for the requested year |
| 9 | `EmptyGenreCards` | No cards exist for the requested genre |
| 10 | `RoundNotStarted` | Action requires a started round |
| 11 | `RoundCompleted` | Round has no cards left / is already finished |
| 12 | `NotAParticipant` | Caller is not a player in the round |
| 13 | `AlreadyReady` | Caller already signalled ready for the round |
| 14 | `NotAuthorized` | Caller lacks the required owner/admin/round role |
| 15 | `AmountExceedsLimit` | Asked for more random cards than exist (e.g. cards-per-round larger than the catalogue) |
| 16 | `LimitMustBeGreaterThanZero` | Random selection over an empty set (no cards added yet) |
| 17 | `NonExistingCard` | No card with the given id |
| 18 | `RoundNotReady` | `finalize_round` called before all answers submitted and before the deadline |
| 19 | `RoundAlreadyFinalized` | `finalize_round` called a second time on an already-finalized round |

### `lyricsflip-nft`

| Code | Name | Meaning |
| --- | --- | --- |
| 1 | `AlreadyInitialized` | Constructor ran on an already-initialized contract (*defensive*) |
| 2 | `NotMinter` | Caller of `mint` is not the configured minter |
| 3 | `TokenAlreadyExists` | Token id collision on mint (*defensive*) |
| 4 | `TokenDoesNotExist` | `owner_of` was called for an unminted token |
