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

## Views for lobbies and catalogue size

| Function | Returns |
| --- | --- |
| `get_cards_count()` | Number of live cards, i.e. added and not removed (`u64`) |
| `get_round_count()` | Total number of rounds created (`u64`) |
| `get_genre_card_count(genre)` | Number of cards in `genre` (`u32`) |
| `get_rounds(start, limit)` | Up to `limit` rounds from round id `start` (ids begin at 1), ascending |
| `get_open_rounds(start, limit)` | Ids of created-but-not-started rounds, oldest first; `start` is an offset |

Both paginated views clamp `limit` to `MAX_PAGE_LIMIT` (50) and return an
empty list once `start` is past the end.

## Card catalogue

Admins manage cards with `add_card`, `add_cards`, `update_card` and
`remove_card`. `add_card`/`add_cards` return the assigned id(s) and ignore the
`card_id` field of the input. Ids are never reused after a removal.

Every card is validated: `title`, `artist` and `lyrics` must be non-empty,
`lyrics` at most `MAX_LYRICS_LEN` (1000) bytes, and `year` between 1900 and
the current year (derived from the ledger timestamp). A card with the same
`title` + `artist` as an existing card is rejected as `DuplicateCard`.

The global, genre, artist and year indexes are each stored as a count plus one
ledger entry per item (`CardAt(i)`, `GenreCardAt((genre, i))`, …) and removals
swap the last item into the gap. Adding a card therefore reads and writes the
same number of entries (13 writes, about 1.9 KB) whether it is the first or
the thousandth card in its genre; see `test::add_card_cost_does_not_grow_with_index_size`.

Rounds are capped at `MAX_ROUND_PLAYERS` (8) players so the per-round player
list stays bounded.

### Batch size

`add_cards` accepts at most `MAX_CARDS_PER_BATCH` (**20**) cards. A full batch
on a fresh catalogue measures about 19.2M instructions, 152 write entries and
24 KB written, against the Mainnet per-transaction limits of 400M
instructions and 200 write entries (as bundled with soroban-sdk 27). Each card
costs roughly 7 new ledger entries, so write entries are the binding limit and
batches above ~25 cards would not fit. `test::add_cards_max_batch_fits_budget`
runs a full batch with Mainnet limits enforced.

## Events

Topics are listed in order after the event name, which soroban-sdk adds as the
first topic (snake_case, e.g. `card_added`). Data is a map of the remaining
fields.

| Event | Emitted by | Topics | Data |
| --- | --- | --- | --- |
| `RoundCreated` | `create_round` | `round_id: u64`, `admin: Address` | `created_time: u64` |
| `RoundJoined` | `join_round` | `round_id: u64`, `player: Address` | `joined_time: u64` |
| `PlayerReady` | `start_round` | `round_id: u64`, `player: Address` | `ready_time: u64` |
| `RoundStarted` | `start_round` (last player ready) | `round_id: u64`, `admin: Address` | `start_time: u64` |
| `CardDrawn` | `next_card` | `round_id: u64` | `index: u32` (position in the round), `card_id: u64` |
| `AnswerSubmitted` | `submit_answer` (first answer per card) | `round_id: u64`, `player: Address` | `correct: bool` |
| `RoundCompleted` | `finalize_round` | `round_id: u64` | `winners: Vec<Address>`, `scores: Map<Address, u64>` |
| `CardAdded` | `add_card`, `add_cards` (one per card) | `card_id: u64`, `genre: Genre` | – |
| `CardUpdated` | `update_card` | `card_id: u64`, `genre: Genre` (new genre) | – |
| `CardRemoved` | `remove_card` | `card_id: u64` | – |
| `RoleUpdated` | `set_role` | `account: Address` | `role: Role`, `enabled: bool` |
| `CardsPerRoundUpdated` | `set_cards_per_round` | – | `value: u32` |

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
| 21 | `RoundFull` | Round already has `MAX_ROUND_PLAYERS` players |
| 26 | `InvalidCardTitle` | Card `title` is empty |
| 27 | `InvalidCardArtist` | Card `artist` is empty |
| 28 | `InvalidCardLyrics` | Card `lyrics` is empty |
| 29 | `InvalidCardYear` | Card `year` is before 1900 or after the current year |
| 30 | `LyricsTooLong` | Card `lyrics` exceed `MAX_LYRICS_LEN` bytes |
| 31 | `DuplicateCard` | Another card already has this `title` + `artist` |
| 32 | `BatchTooLarge` | `add_cards` got more than `MAX_CARDS_PER_BATCH` cards |

### `lyricsflip-nft`

| Code | Name | Meaning |
| --- | --- | --- |
| 1 | `AlreadyInitialized` | Constructor ran on an already-initialized contract (*defensive*) |
| 2 | `NotMinter` | Caller of `mint` is not the configured minter |
| 3 | `TokenAlreadyExists` | Token id collision on mint (*defensive*) |
| 4 | `TokenDoesNotExist` | `owner_of` was called for an unminted token |
