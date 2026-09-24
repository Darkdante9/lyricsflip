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

When a round is finalized it is marked `is_completed`, and `end_time` is set
to the finalization time, or kept at the deadline if the round timed out.

## Round lifecycle, limits and timing

- **Player cap:** `join_round` fails with `RoundFull` once a round has
  `get_max_players()` players (default `8`; the owner can change it with
  `set_max_players(caller, value)`, minimum 2).
- **Round deadline:** a round's `end_time` is `start_time + 300s`. After it,
  `submit_answer` fails with `RoundCompleted`, and the round can be finalized.
- **Card answer window:** players have `CARD_ANSWER_WINDOW_SECONDS` (15s) after
  `next_card` to answer. **Late answers are accepted but scored as wrong**
  (they break the streak), so the player still counts as having answered and
  the round can finalize early.
- **Leaving:** `leave_round(caller, round_id)` removes a non-admin player
  before the round starts, clears their ready flag, refunds their wager and
  emits `RoundLeft { round_id, player, refunded }`.
- **Cancelling:** `cancel_round(caller, round_id)` works before start. The
  round admin can call it at any time; anyone else can call it once the lobby
  has been open for `LOBBY_TIMEOUT_SECONDS` (600s). It sets `is_cancelled`,
  drops the round from `get_open_rounds`, refunds every player and emits
  `RoundCancelled { round_id, cancelled_by, refunded_players, refund_per_player }`.
  A cancelled round can't be joined, started, or finalized (`RoundCancelled`).

Wagers are not escrowed yet (LF-013), so refunds currently report
`round.wager_amount` without moving tokens. The token transfer goes into
`refund_wager` once escrow lands.

## NFT rewards

The game contract is the NFT contract's minter. The owner registers the NFT
contract with `set_nft_contract(caller, nft_contract)`; players then call
`claim_reward(caller, milestone)`, and the game contract mints through a
cross-contract call to `mint`. Each milestone can be claimed once per player
(`MilestoneAlreadyClaimed`); `is_milestone_claimed(player, milestone)` reports
whether it has been claimed.

| Milestone | Value | Requirement |
| --- | --- | --- |
| `FirstWin` | 0 | `rounds_won >= 1` |
| `Streak5` | 1 | `max_streak >= 5` |
| `TenWins` | 2 | `rounds_won >= 10` |

### `lyricsflip-nft` interface

The contract follows the [SEP-0050](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md)
non-fungible interface, with method names and events matching OpenZeppelin's
`stellar-non-fungible`. It stays hand-written rather than depending on that
library, to avoid tying the workspace's `soroban-sdk` version to the
library's.

| Function | Description |
| --- | --- |
| `mint(caller, recipient) -> u128` | Minter-only; mints the next token id |
| `owner_of(token_id) -> Address` | Owner of a token |
| `balance(owner) -> u32` | Number of tokens held by `owner` |
| `transfer(from, to, token_id)` | Owner transfers a token (auth: `from`) |
| `transfer_from(spender, from, to, token_id)` | Approved spender or operator transfers (auth: `spender`) |
| `approve(approver, approved, token_id, live_until_ledger)` | Owner or operator approves one spender for a token; `0` revokes |
| `approve_for_all(owner, operator, live_until_ledger)` | Approves an operator for all of `owner`'s tokens; `0` revokes |
| `get_approved(token_id) -> Option<Address>` | Current, unexpired token approval |
| `is_approved_for_all(owner, operator) -> bool` | Whether `operator` is an unexpired operator |
| `token_name()`, `token_symbol()`, `base_uri()`, `token_count()` | Metadata |
| `token_uri(token_id) -> String` | `base_uri` followed by the token id; `TokenDoesNotExist` for unminted tokens |
| `set_base_uri(caller, base_uri)` | Owner-only; at most 200 bytes |
| `set_minter(caller, new_minter)` | Owner-only; the old minter can no longer mint |
| `owner()`, `minter()`, `pending_owner()` | Administration views |

Per-token metadata follows the JSON template in `metadata/template.json`, served
at `<base_uri><token_id>`.

Events: `NftMinted`, `Transfer { from, to, token_id }`,
`Approve { approver, token_id, approved, live_until_ledger }` and
`ApproveForAll { owner, operator, live_until_ledger }`, `MinterUpdated`. A
transfer clears the token's approval.

## Ownership, roles and upgrades

Both contracts support a two-step ownership transfer: the owner calls
`transfer_ownership(caller, new_owner)`, then the new owner calls
`accept_ownership(caller)` (`OwnershipTransferStarted` / `OwnershipTransferred`
events; `owner()` and `pending_owner()` views). On `lyricsflip`, the new owner
is also granted the admin role, and `set_role` is owner-only regardless of the
owner's own admin flag, so the owner cannot lock themselves out. `set_role`
emits `RoleUpdated { address, role, enabled }`.

Both contracts expose `version() -> u32` and an owner-only
`upgrade(caller, new_wasm_hash)` that replaces the code in place and keeps all
storage (cards, stats, NFTs). Bump `VERSION` in the contract on each release,
then:

```bash
cargo build --target wasm32v1-none --release
HASH=$(stellar contract upload \
  --wasm target/wasm32v1-none/release/lyricsflip.wasm \
  --source <OWNER_IDENTITY> --network testnet)
stellar contract invoke --id <LYRICSFLIP_CONTRACT_ID> \
  --source <OWNER_IDENTITY> --network testnet \
  -- upgrade --caller <OWNER_ADDRESS> --new_wasm_hash $HASH
```

A release that changes the layout of stored data must migrate it; there is no
storage schema version key yet, so add one alongside the first such change.
`fixtures/upgrade_v2.wasm` (built from `contracts/upgrade-fixture`) is the
stand-in new code used by the upgrade tests.

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
| 18 | `RoundNotReady` | `finalize_round` before all answers are in or the deadline has passed |
| 19 | `RoundAlreadyFinalized` | Round was already finalized |
| 20 | `RoundCancelled` | Round was cancelled |
| 21 | `RoundFull` | Round already has `max_players` players |
| 22 | `InvalidMaxPlayers` | `set_max_players` was called with a value below 2 |
| 23 | `NftContractNotSet` | `claim_reward` before the owner set the NFT contract |
| 24 | `MilestoneNotReached` | Player's stats don't meet the milestone yet |
| 25 | `MilestoneAlreadyClaimed` | Player already claimed that milestone |
| 26 | `NotPendingOwner` | Caller of `accept_ownership` is not the pending owner |

### `lyricsflip-nft`

| Code | Name | Meaning |
| --- | --- | --- |
| 1 | `AlreadyInitialized` | Constructor ran on an already-initialized contract (*defensive*) |
| 2 | `NotMinter` | Caller of `mint` is not the configured minter |
| 3 | `TokenAlreadyExists` | Token id collision on mint (*defensive*) |
| 4 | `TokenDoesNotExist` | `owner_of` or `token_uri` was called for an unminted token |
| 5 | `IncorrectOwner` | `from` does not own the token being transferred |
| 6 | `InsufficientApproval` | Spender/approver is neither the owner nor approved |
| 7 | `InvalidLiveUntilLedger` | Approval expiry is already in the past |
| 8 | `NotOwner` | Caller of an owner-only function is not the owner |
| 9 | `NotPendingOwner` | Caller of `accept_ownership` is not the pending owner |
| 10 | `BaseUriTooLong` | `base_uri` is longer than 200 bytes |
