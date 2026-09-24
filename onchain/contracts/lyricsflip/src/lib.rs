#![no_std]

mod errors;
mod events;
mod types;

#[cfg(test)]
mod test;

pub use errors::Error;
pub use events::{
    OwnershipTransferStarted, OwnershipTransferred, PlayerReady, RewardClaimed, RoleUpdated,
    RoundCancelled, RoundCompleted, RoundCreated, RoundJoined, RoundLeft, RoundStarted,
};
pub use types::{Answer, Card, DataKey, Genre, Milestone, PlayerStats, QuestionCard, Role, Round};

use soroban_sdk::{
    contract, contractclient, contractimpl, panic_with_error, Address, Bytes, BytesN, Env, Map,
    String, Vec,
};

/// Bumped on every release that changes the contract's code; see `upgrade`.
pub const VERSION: u32 = 1;

const DEFAULT_ROUND_DURATION_SECONDS: u64 = 300;

/// Default cap on players per round when the owner hasn't set `max_players`.
pub const DEFAULT_MAX_PLAYERS: u32 = 8;

/// Seconds a player has to answer after a card is flipped (README card-flip
/// rule). Answers submitted after the window are accepted but scored as
/// wrong, so every player can still complete the round.
pub const CARD_ANSWER_WINDOW_SECONDS: u64 = 15;

/// Seconds after creation after which anyone may cancel a round that never
/// started.
pub const LOBBY_TIMEOUT_SECONDS: u64 = 600;

/// The subset of the `lyricsflip-nft` interface the game contract calls.
#[contractclient(name = "NftClient")]
#[allow(dead_code)]
pub trait NftInterface {
    fn mint(env: Env, caller: Address, recipient: Address) -> u128;
}

/// Upper bound on the page size of the paginated list views (`get_rounds`,
/// `get_open_rounds`). Larger `limit` values are clamped to this.
pub const MAX_PAGE_LIMIT: u32 = 50;

#[contract]
pub struct LyricsFlip;

#[contractimpl]
impl LyricsFlip {
    /// Ported from the Cairo `#[constructor]`: sets the owner and grants them
    /// the (only) admin role.
    pub fn __constructor(env: Env, owner: Address) {
        if env.storage().instance().has(&DataKey::Owner) {
            panic_with_error!(env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Admin(owner), &true);
    }

    // ---- Views ----

    pub fn get_round(env: Env, round_id: u64) -> Round {
        Self::read_round(&env, round_id)
    }

    pub fn get_round_cards(env: Env, round_id: u64) -> Vec<u64> {
        Self::read_round_cards(&env, round_id)
    }

    pub fn get_round_players(env: Env, round_id: u64) -> Vec<Address> {
        Self::read_round_players(&env, round_id)
    }

    pub fn get_players_round_count(env: Env, round_id: u64) -> u32 {
        Self::read_round_players(&env, round_id).len()
    }

    pub fn get_round_scores(env: Env, round_id: u64) -> Map<Address, u64> {
        let players = Self::read_round_players(&env, round_id);
        let mut scores = Self::read_round_scores(&env, round_id);

        for player in players.iter() {
            if scores.get(player.clone()).is_none() {
                scores.set(player.clone(), 0u64);
            }
        }

        scores
    }

    pub fn finalize_round(env: Env, caller: Address, round_id: u64) {
        caller.require_auth();
        let round = Self::read_round(&env, round_id);
        if !Self::is_round_player(&env, round_id, &caller) {
            panic_with_error!(env, Error::NotAParticipant);
        }
        if round.is_cancelled {
            panic_with_error!(env, Error::RoundCancelled);
        }
        if env
            .storage()
            .persistent()
            .get(&DataKey::RoundFinalized(round_id))
            .unwrap_or(false)
        {
            panic_with_error!(env, Error::RoundAlreadyFinalized);
        }

        let now = env.ledger().timestamp();
        let is_all_cards_answered = Self::are_all_required_answers_submitted(&env, round_id);
        let is_past_deadline = round.end_time != 0 && now >= round.end_time;
        if !is_all_cards_answered && !is_past_deadline {
            panic_with_error!(env, Error::RoundNotReady);
        }

        // Record when the round actually ended: now if everyone finished
        // early, otherwise the (already passed) deadline.
        let mut round = round;
        round.is_completed = true;
        round.end_time = if is_past_deadline {
            round.end_time
        } else {
            now
        };
        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);

        let winners = Self::determine_round_winners(&env, round_id);
        if winners.len() > 0 {
            for player in Self::read_round_players(&env, round_id).iter() {
                let mut is_winner = false;
                for winner in winners.iter() {
                    if winner == player {
                        is_winner = true;
                        break;
                    }
                }
                if is_winner {
                    let mut stats = Self::get_player_stat(env.clone(), player.clone());
                    stats.rounds_won += 1;
                    env.storage()
                        .persistent()
                        .set(&DataKey::PlayerStats(player), &stats);
                }
            }
        }

        let scores = Self::get_round_scores(env.clone(), round_id);
        RoundCompleted {
            round_id,
            winners: winners.clone(),
            scores,
        }
        .publish(&env);

        env.storage()
            .persistent()
            .set(&DataKey::RoundFinalized(round_id), &true);
    }

    pub fn get_cards_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0)
    }

    pub fn get_round_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::RoundCount)
            .unwrap_or(0)
    }

    pub fn get_genre_card_count(env: Env, genre: Genre) -> u32 {
        env.storage()
            .persistent()
            .get::<_, Vec<u64>>(&DataKey::GenreCards(genre))
            .map(|ids| ids.len())
            .unwrap_or(0)
    }

    /// Returns up to `limit` rounds (clamped to `MAX_PAGE_LIMIT`) starting at
    /// round id `start`, in ascending id order. Round ids begin at 1, so a
    /// `start` of 0 is treated as 1. Returns an empty list past the end.
    pub fn get_rounds(env: Env, start: u64, limit: u32) -> Vec<Round> {
        let round_count = Self::get_round_count(env.clone());
        let limit = limit.min(MAX_PAGE_LIMIT) as u64;
        let mut rounds: Vec<Round> = Vec::new(&env);

        let mut round_id = start.max(1);
        while round_id <= round_count && (rounds.len() as u64) < limit {
            rounds.push_back(Self::read_round(&env, round_id));
            round_id += 1;
        }
        rounds
    }

    /// Ids of rounds that are created but not yet started (i.e. joinable),
    /// oldest first. `start` is an offset into that list and `limit` is
    /// clamped to `MAX_PAGE_LIMIT`.
    pub fn get_open_rounds(env: Env, start: u32, limit: u32) -> Vec<u64> {
        let open = Self::read_open_rounds(&env);
        let end = start
            .saturating_add(limit.min(MAX_PAGE_LIMIT))
            .min(open.len());
        if start >= end {
            return Vec::new(&env);
        }
        open.slice(start..end)
    }

    pub fn get_max_players(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxPlayers)
            .unwrap_or(DEFAULT_MAX_PLAYERS)
    }

    pub fn get_nft_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::NftContract)
    }

    pub fn is_milestone_claimed(env: Env, player: Address, milestone: Milestone) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::MilestoneClaimed((player, milestone)))
            .unwrap_or(false)
    }

    pub fn get_cards_per_round(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CardsPerRound)
            .unwrap_or(0)
    }

    pub fn get_card(env: Env, card_id: u64) -> Card {
        env.storage()
            .persistent()
            .get(&DataKey::Card(card_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::NonExistingCard))
    }

    pub fn get_cards_of_genre(env: Env, genre: Genre, seed: u64) -> Vec<Card> {
        let ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::GenreCards(genre))
            .unwrap_or(Vec::new(&env));
        let limit = ids.len() as u64;
        if limit == 0 {
            panic_with_error!(env, Error::EmptyGenreCards);
        }
        let amount = Self::get_cards_per_round(env.clone()) as u64;
        let indices = Self::get_random_numbers(&env, seed, amount, limit, true);

        let mut cards: Vec<Card> = Vec::new(&env);
        for idx in indices.iter() {
            let card_id = ids.get(idx as u32).unwrap();
            cards.push_back(Self::get_card(env.clone(), card_id));
        }
        cards
    }

    pub fn get_cards_of_artist(env: Env, artist: String, seed: u64) -> Vec<Card> {
        let ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ArtistCards(artist))
            .unwrap_or(Vec::new(&env));
        let limit = ids.len() as u64;
        if limit == 0 {
            panic_with_error!(env, Error::ArtistCardsIsZero);
        }
        let amount = Self::get_cards_per_round(env.clone()) as u64;
        let indices = Self::get_random_numbers(&env, seed, amount, limit, true);

        let mut cards: Vec<Card> = Vec::new(&env);
        for idx in indices.iter() {
            let card_id = ids.get(idx as u32).unwrap();
            cards.push_back(Self::get_card(env.clone(), card_id));
        }
        cards
    }

    pub fn get_cards_of_a_year(env: Env, year: u64, seed: u64) -> Vec<Card> {
        let ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::YearCards(year))
            .unwrap_or(Vec::new(&env));
        let limit = ids.len() as u64;
        if limit == 0 {
            panic_with_error!(env, Error::EmptyYearCards);
        }
        let amount = Self::get_cards_per_round(env.clone()) as u64;
        let indices = Self::get_random_numbers(&env, seed, amount, limit, true);

        let mut cards: Vec<Card> = Vec::new(&env);
        for idx in indices.iter() {
            let card_id = ids.get(idx as u32).unwrap();
            cards.push_back(Self::get_card(env.clone(), card_id));
        }
        cards
    }

    pub fn get_player_stat(env: Env, player: Address) -> PlayerStats {
        env.storage()
            .persistent()
            .get(&DataKey::PlayerStats(player))
            .unwrap_or(PlayerStats::zero())
    }

    pub fn is_admin(env: Env, role: Role, address: Address) -> bool {
        let Role::Admin = role;
        env.storage()
            .instance()
            .get(&DataKey::Admin(address))
            .unwrap_or(false)
    }

    // ---- Mutations ----

    pub fn create_round(env: Env, caller: Address, genre: Option<Genre>, seed: u64) -> u64 {
        caller.require_auth();
        let genre = match genre {
            Some(g) => g,
            None => panic_with_error!(env, Error::NonExistingGenre),
        };

        let amount = Self::get_cards_per_round(env.clone()) as u64;
        let cards_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0);
        let cards = Self::get_random_numbers(&env, seed, amount, cards_count, false);

        let round_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::RoundCount)
            .unwrap_or(0);
        let round_id = round_count + 1;
        env.storage()
            .instance()
            .set(&DataKey::RoundCount, &round_id);

        let round = Round {
            round_id,
            admin: caller.clone(),
            genre,
            wager_amount: 0,
            start_time: 0,
            is_started: false,
            is_completed: false,
            end_time: 0,
            next_card_index: 0,
            is_cancelled: false,
        };

        let mut players: Vec<Address> = Vec::new(&env);
        players.push_back(caller.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RoundPlayers(round_id), &players);
        env.storage()
            .persistent()
            .set(&DataKey::RoundCards(round_id), &cards);
        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);

        env.storage().persistent().set(
            &DataKey::RoundCreatedAt(round_id),
            &env.ledger().timestamp(),
        );

        let mut open = Self::read_open_rounds(&env);
        open.push_back(round_id);
        env.storage().persistent().set(&DataKey::OpenRounds, &open);

        RoundCreated {
            round_id,
            admin: caller,
            created_time: env.ledger().timestamp(),
        }
        .publish(&env);

        round_id
    }

    pub fn start_round(env: Env, caller: Address, round_id: u64) {
        caller.require_auth();
        let mut round = Self::read_round(&env, round_id);
        if round.is_cancelled {
            panic_with_error!(env, Error::RoundCancelled);
        }

        let is_round_admin = round.admin == caller;
        let is_participant = Self::is_round_player(&env, round_id, &caller);
        if !is_round_admin && !is_participant {
            panic_with_error!(env, Error::NotAuthorized);
        }

        let ready_key = DataKey::RoundReady((round_id, caller.clone()));
        let already_ready: bool = env.storage().persistent().get(&ready_key).unwrap_or(false);
        if already_ready {
            panic_with_error!(env, Error::AlreadyReady);
        }

        let players = Self::read_round_players(&env, round_id);

        env.storage().persistent().set(&ready_key, &true);

        let ready_count_key = DataKey::RoundReadyCount(round_id);
        let ready_count: u32 = env
            .storage()
            .persistent()
            .get(&ready_count_key)
            .unwrap_or(0)
            + 1;
        env.storage()
            .persistent()
            .set(&ready_count_key, &ready_count);

        PlayerReady {
            round_id,
            player: caller,
            ready_time: env.ledger().timestamp(),
        }
        .publish(&env);

        // Increment total_rounds only when the round actually starts (all
        // players ready). Individual ready calls must not inflate the count,
        // and a round that never reaches ready_count == players.len() must
        // not change it.
        if ready_count == players.len() {
            for player in players.iter() {
                let mut stats = Self::get_player_stat(env.clone(), player.clone());
                stats.total_rounds += 1;
                env.storage()
                    .persistent()
                    .set(&DataKey::PlayerStats(player), &stats);
            }

            let start_time = env.ledger().timestamp();
            round.start_time = start_time;
            round.end_time = start_time + DEFAULT_ROUND_DURATION_SECONDS;
            round.is_started = true;
            env.storage()
                .persistent()
                .set(&DataKey::Round(round_id), &round);

            Self::remove_open_round(&env, round_id);

            RoundStarted {
                round_id,
                admin: round.admin,
                start_time,
            }
            .publish(&env);
        }
    }

    pub fn join_round(env: Env, caller: Address, round_id: u64) {
        caller.require_auth();
        let round = Self::read_round(&env, round_id);

        if Self::is_round_player(&env, round_id, &caller) {
            panic_with_error!(env, Error::RoundAlreadyJoined);
        }
        if round.is_cancelled {
            panic_with_error!(env, Error::RoundCancelled);
        }
        if round.is_started {
            panic_with_error!(env, Error::RoundAlreadyStarted);
        }

        let mut players = Self::read_round_players(&env, round_id);
        if players.len() >= Self::get_max_players(env.clone()) {
            panic_with_error!(env, Error::RoundFull);
        }
        players.push_back(caller.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RoundPlayers(round_id), &players);

        RoundJoined {
            round_id,
            player: caller,
            joined_time: env.ledger().timestamp(),
        }
        .publish(&env);
    }

    /// Leaves a round that hasn't started yet and refunds the caller's wager.
    /// The round admin can't leave; they cancel the round instead.
    pub fn leave_round(env: Env, caller: Address, round_id: u64) {
        caller.require_auth();
        let round = Self::read_round(&env, round_id);
        Self::assert_round_pending(&env, &round);
        if round.admin == caller {
            panic_with_error!(env, Error::NotAuthorized);
        }

        let mut players = Self::read_round_players(&env, round_id);
        let idx = players
            .first_index_of(&caller)
            .unwrap_or_else(|| panic_with_error!(env, Error::NotAParticipant));
        players.remove(idx);
        env.storage()
            .persistent()
            .set(&DataKey::RoundPlayers(round_id), &players);

        let ready_key = DataKey::RoundReady((round_id, caller.clone()));
        if env.storage().persistent().get(&ready_key).unwrap_or(false) {
            env.storage().persistent().remove(&ready_key);
            let ready_count_key = DataKey::RoundReadyCount(round_id);
            let ready_count: u32 = env
                .storage()
                .persistent()
                .get(&ready_count_key)
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&ready_count_key, &ready_count.saturating_sub(1));
        }

        let refunded = Self::refund_wager(&env, &round, &caller);
        RoundLeft {
            round_id,
            player: caller,
            refunded,
        }
        .publish(&env);
    }

    /// Cancels a round that hasn't started and refunds every player. Callable
    /// by the round admin at any time before start, or by anyone once the
    /// lobby has been open for `LOBBY_TIMEOUT_SECONDS`.
    pub fn cancel_round(env: Env, caller: Address, round_id: u64) {
        caller.require_auth();
        let mut round = Self::read_round(&env, round_id);
        Self::assert_round_pending(&env, &round);

        if round.admin != caller {
            let created_at: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::RoundCreatedAt(round_id))
                .unwrap_or(0);
            if env.ledger().timestamp() < created_at + LOBBY_TIMEOUT_SECONDS {
                panic_with_error!(env, Error::NotAuthorized);
            }
        }

        round.is_cancelled = true;
        round.end_time = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);
        Self::remove_open_round(&env, round_id);

        let players = Self::read_round_players(&env, round_id);
        let mut refund_per_player = 0;
        for player in players.iter() {
            refund_per_player = Self::refund_wager(&env, &round, &player);
        }

        RoundCancelled {
            round_id,
            cancelled_by: caller,
            refunded_players: players,
            refund_per_player,
        }
        .publish(&env);
    }

    pub fn next_card(env: Env, round_id: u64) -> Card {
        let mut round = Self::read_round(&env, round_id);
        if !round.is_started {
            panic_with_error!(env, Error::RoundNotStarted);
        }
        if round.is_completed {
            panic_with_error!(env, Error::RoundCompleted);
        }

        let round_cards = Self::read_round_cards(&env, round_id);
        let card_id = round_cards
            .get(round.next_card_index)
            .unwrap_or_else(|| panic_with_error!(env, Error::RoundCompleted));
        let card = Self::get_card(env.clone(), card_id);
        env.storage().persistent().set(
            &DataKey::RoundCardStartedAt((round_id, card_id)),
            &env.ledger().timestamp(),
        );

        // The round is marked completed by `finalize_round`, so players can
        // still answer the last card after it is drawn.
        round.next_card_index += 1;
        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);

        card
    }

    pub fn set_cards_per_round(env: Env, caller: Address, value: u32) {
        caller.require_auth();
        Self::assert_admin(&env, &caller);
        if value == 0 {
            panic_with_error!(env, Error::InvalidCardsPerRound);
        }
        env.storage()
            .instance()
            .set(&DataKey::CardsPerRound, &value);
    }

    pub fn add_card(env: Env, caller: Address, card: Card) {
        caller.require_auth();
        Self::assert_admin(&env, &caller);

        let cards_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0);
        let card_id = cards_count + 1;

        let mut artist_cards: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ArtistCards(card.artist.clone()))
            .unwrap_or(Vec::new(&env));
        artist_cards.push_back(card_id);
        env.storage()
            .persistent()
            .set(&DataKey::ArtistCards(card.artist.clone()), &artist_cards);

        let mut genre_cards: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::GenreCards(card.genre))
            .unwrap_or(Vec::new(&env));
        genre_cards.push_back(card_id);
        env.storage()
            .persistent()
            .set(&DataKey::GenreCards(card.genre), &genre_cards);

        let mut year_cards: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::YearCards(card.year))
            .unwrap_or(Vec::new(&env));
        year_cards.push_back(card_id);
        env.storage()
            .persistent()
            .set(&DataKey::YearCards(card.year), &year_cards);

        env.storage()
            .persistent()
            .set(&DataKey::Card(card_id), &card);
        env.storage().instance().set(&DataKey::CardsCount, &card_id);
    }

    /// Owner-only. The owner may manage roles even after revoking their own
    /// admin flag, so they can never lock themselves out.
    pub fn set_role(env: Env, caller: Address, recipient: Address, role: Role, is_enable: bool) {
        caller.require_auth();
        Self::assert_owner(&env, &caller);
        let Role::Admin = role;
        env.storage()
            .instance()
            .set(&DataKey::Admin(recipient.clone()), &is_enable);
        RoleUpdated {
            address: recipient,
            role,
            enabled: is_enable,
        }
        .publish(&env);
    }

    pub fn owner(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Owner).unwrap()
    }

    pub fn pending_owner(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingOwner)
    }

    pub fn version() -> u32 {
        VERSION
    }

    /// Step one of an ownership transfer; `new_owner` must then call
    /// `accept_ownership`. Calling again replaces the pending owner.
    pub fn transfer_ownership(env: Env, caller: Address, new_owner: Address) {
        caller.require_auth();
        Self::assert_owner(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::PendingOwner, &new_owner);
        OwnershipTransferStarted {
            owner: caller,
            pending_owner: new_owner,
        }
        .publish(&env);
    }

    /// Completes an ownership transfer and grants the new owner the admin
    /// role. The previous owner keeps any admin role they had.
    pub fn accept_ownership(env: Env, caller: Address) {
        caller.require_auth();
        if Self::pending_owner(env.clone()) != Some(caller.clone()) {
            panic_with_error!(env, Error::NotPendingOwner);
        }
        let old_owner = Self::owner(env.clone());
        env.storage().instance().set(&DataKey::Owner, &caller);
        env.storage().instance().remove(&DataKey::PendingOwner);
        env.storage()
            .instance()
            .set(&DataKey::Admin(caller.clone()), &true);
        OwnershipTransferred {
            old_owner,
            new_owner: caller,
        }
        .publish(&env);
    }

    /// Owner-only. Replaces this contract's code in place, keeping all
    /// storage.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        caller.require_auth();
        Self::assert_owner(&env, &caller);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    pub fn submit_answer(env: Env, caller: Address, round_id: u64, answer: Answer) -> bool {
        caller.require_auth();
        if !Self::is_round_player(&env, round_id, &caller) {
            panic_with_error!(env, Error::NotAParticipant);
        }

        let round = Self::read_round(&env, round_id);
        if !round.is_started {
            panic_with_error!(env, Error::RoundNotStarted);
        }
        if round.is_completed || env.ledger().timestamp() >= round.end_time {
            panic_with_error!(env, Error::RoundCompleted);
        }
        if round.next_card_index == 0 {
            panic_with_error!(env, Error::RoundNotStarted);
        }

        let current_index = round.next_card_index - 1;
        let round_cards = Self::read_round_cards(&env, round_id);
        let current_card_id = round_cards.get(current_index).unwrap();
        let current_card = Self::get_card(env.clone(), current_card_id);

        let answered_key =
            DataKey::RoundPlayerAnswered((round_id, caller.clone(), current_card_id));
        if env
            .storage()
            .persistent()
            .get(&answered_key)
            .unwrap_or(false)
        {
            return false;
        }
        env.storage().persistent().set(&answered_key, &true);

        let answer_started_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RoundCardStartedAt((round_id, current_card_id)))
            .unwrap_or(round.start_time);
        let answer_time = env.ledger().timestamp().saturating_sub(answer_started_at);
        let answer_times = Self::read_round_answer_times(&env, round_id);
        let mut answer_times = answer_times;
        let total_time = answer_times.get(caller.clone()).unwrap_or(0u64);
        answer_times.set(caller.clone(), total_time + answer_time);
        env.storage()
            .persistent()
            .set(&DataKey::RoundAnswerTimes(round_id), &answer_times);

        // Answers after the card window are scored as wrong (not rejected),
        // so the player still counts as having answered the card.
        let is_answer_correct = answer_time <= CARD_ANSWER_WINDOW_SECONDS
            && match answer {
                Answer::Artist(value) => value == current_card.artist,
                Answer::Year(value) => value == current_card.year,
                Answer::Title(value) => value == current_card.title,
            };

        let mut scores = Self::read_round_scores(&env, round_id);
        if is_answer_correct {
            let score = scores.get(caller.clone()).unwrap_or(0u64);
            scores.set(caller.clone(), score + 1);
            env.storage()
                .persistent()
                .set(&DataKey::RoundScores(round_id), &scores);
        }

        let mut stats = Self::get_player_stat(env.clone(), caller.clone());
        if is_answer_correct {
            stats.current_streak += 1;
            if stats.current_streak > stats.max_streak {
                stats.max_streak = stats.current_streak;
            }
        } else {
            stats.current_streak = 0;
        }
        env.storage()
            .persistent()
            .set(&DataKey::PlayerStats(caller), &stats);

        is_answer_correct
    }

    pub fn set_max_players(env: Env, caller: Address, value: u32) {
        caller.require_auth();
        Self::assert_owner(&env, &caller);
        if value < 2 {
            panic_with_error!(env, Error::InvalidMaxPlayers);
        }
        env.storage().instance().set(&DataKey::MaxPlayers, &value);
    }

    pub fn set_nft_contract(env: Env, caller: Address, nft_contract: Address) {
        caller.require_auth();
        Self::assert_owner(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::NftContract, &nft_contract);
    }

    /// Mints the NFT for `milestone` to `caller` through a cross-contract
    /// call. This contract must be the NFT contract's minter. Each milestone
    /// can be claimed once per player.
    pub fn claim_reward(env: Env, caller: Address, milestone: Milestone) -> u128 {
        caller.require_auth();
        let nft_contract = Self::get_nft_contract(env.clone())
            .unwrap_or_else(|| panic_with_error!(env, Error::NftContractNotSet));

        let claimed_key = DataKey::MilestoneClaimed((caller.clone(), milestone));
        if env.storage().persistent().has(&claimed_key) {
            panic_with_error!(env, Error::MilestoneAlreadyClaimed);
        }

        let stats = Self::get_player_stat(env.clone(), caller.clone());
        let reached = match milestone {
            Milestone::FirstWin => stats.rounds_won >= 1,
            Milestone::Streak5 => stats.max_streak >= 5,
            Milestone::TenWins => stats.rounds_won >= 10,
        };
        if !reached {
            panic_with_error!(env, Error::MilestoneNotReached);
        }

        env.storage().persistent().set(&claimed_key, &true);
        let token_id =
            NftClient::new(&env, &nft_contract).mint(&env.current_contract_address(), &caller);

        RewardClaimed {
            player: caller,
            milestone,
            token_id,
        }
        .publish(&env);

        token_id
    }

    pub fn build_question_card(env: Env, card: Card, seed: u64) -> QuestionCard {
        let cards_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0);
        let random_ids = Self::get_random_numbers(&env, seed, 10, cards_count, false);

        let mut false_answers: Vec<String> = Vec::new(&env);
        for id in random_ids.iter() {
            if false_answers.len() >= 3 {
                break;
            }
            let candidate = Self::get_card(env.clone(), id);
            if candidate.title != card.title
                && !Self::contains_string(&false_answers, &candidate.title)
            {
                false_answers.push_back(candidate.title.clone());
            }
        }

        let mut extra_seed = seed + 1;
        while false_answers.len() < 3 {
            let ids = Self::get_random_numbers(&env, extra_seed, 1, cards_count, false);
            let id = ids.get(0).unwrap();
            let candidate = Self::get_card(env.clone(), id);
            if candidate.title != card.title
                && !Self::contains_string(&false_answers, &candidate.title)
            {
                false_answers.push_back(candidate.title.clone());
            }
            extra_seed += 1;
        }

        let mut options: Vec<String> = Vec::new(&env);
        options.push_back(card.title.clone());
        for answer in false_answers.iter() {
            options.push_back(answer.clone());
        }

        let shuffled = Self::shuffle_strings(&env, options, seed);

        QuestionCard {
            lyric: card.lyrics.clone(),
            timestamp: env.ledger().timestamp(),
            option_one: shuffled.get(0).unwrap(),
            option_two: shuffled.get(1).unwrap(),
            option_three: shuffled.get(2).unwrap(),
            option_four: shuffled.get(3).unwrap(),
        }
    }

    // ---- Internal helpers ----

    fn read_round(env: &Env, round_id: u64) -> Round {
        env.storage()
            .persistent()
            .get(&DataKey::Round(round_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::NonExistingRound))
    }

    fn read_round_cards(env: &Env, round_id: u64) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::RoundCards(round_id))
            .unwrap_or(Vec::new(env))
    }

    fn read_round_players(env: &Env, round_id: u64) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::RoundPlayers(round_id))
            .unwrap_or(Vec::new(env))
    }

    fn read_round_scores(env: &Env, round_id: u64) -> Map<Address, u64> {
        env.storage()
            .persistent()
            .get(&DataKey::RoundScores(round_id))
            .unwrap_or(Map::new(env))
    }

    fn read_round_answer_times(env: &Env, round_id: u64) -> Map<Address, u64> {
        env.storage()
            .persistent()
            .get(&DataKey::RoundAnswerTimes(round_id))
            .unwrap_or(Map::new(env))
    }

    fn are_all_required_answers_submitted(env: &Env, round_id: u64) -> bool {
        let players = Self::read_round_players(env, round_id);
        let round_cards = Self::read_round_cards(env, round_id);
        if players.is_empty() || round_cards.is_empty() {
            return false;
        }

        for player in players.iter() {
            for card_id in round_cards.iter() {
                let answered: bool = env
                    .storage()
                    .persistent()
                    .get(&DataKey::RoundPlayerAnswered((
                        round_id,
                        player.clone(),
                        card_id,
                    )))
                    .unwrap_or(false);
                if !answered {
                    return false;
                }
            }
        }

        true
    }

    fn determine_round_winners(env: &Env, round_id: u64) -> Vec<Address> {
        let players = Self::read_round_players(env, round_id);
        let scores = Self::read_round_scores(env, round_id);
        let answer_times = Self::read_round_answer_times(env, round_id);

        let mut winners: Vec<Address> = Vec::new(env);
        let mut max_score: u64 = 0;
        let mut best_time: Option<u64> = None;

        for player in players.iter() {
            let score = scores.get(player.clone()).unwrap_or(0u64);
            if score == 0 && max_score == 0 && winners.len() == 0 {
                continue;
            }

            if score > max_score {
                max_score = score;
                winners = Vec::new(env);
                winners.push_back(player.clone());
                best_time = Some(answer_times.get(player.clone()).unwrap_or(0u64));
                continue;
            }

            if score != max_score {
                continue;
            }

            let time = answer_times.get(player.clone()).unwrap_or(0u64);
            match best_time {
                Some(current_best) => {
                    if time < current_best {
                        winners = Vec::new(env);
                        winners.push_back(player.clone());
                        best_time = Some(time);
                    } else if time == current_best {
                        winners.push_back(player.clone());
                    }
                }
                None => {
                    winners.push_back(player.clone());
                    best_time = Some(time);
                }
            }
        }

        if max_score == 0 {
            winners = Vec::new(env);
        }
        winners
    }

    fn read_open_rounds(env: &Env) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::OpenRounds)
            .unwrap_or(Vec::new(env))
    }

    fn is_round_player(env: &Env, round_id: u64, player: &Address) -> bool {
        let players = Self::read_round_players(env, round_id);
        for p in players.iter() {
            if p == *player {
                return true;
            }
        }
        false
    }

    fn remove_open_round(env: &Env, round_id: u64) {
        let mut open = Self::read_open_rounds(env);
        if let Some(idx) = open.first_index_of(round_id) {
            open.remove(idx);
            env.storage().persistent().set(&DataKey::OpenRounds, &open);
        }
    }

    fn assert_round_pending(env: &Env, round: &Round) {
        if round.is_cancelled {
            panic_with_error!(env, Error::RoundCancelled);
        }
        if round.is_started {
            panic_with_error!(env, Error::RoundAlreadyStarted);
        }
    }

    /// Returns the wager refunded to `player`. Wagers are not escrowed yet
    /// (see LF-013), so this only reports `round.wager_amount`; once escrow
    /// lands, the token transfer back to `player` belongs here.
    fn refund_wager(_env: &Env, round: &Round, _player: &Address) -> i128 {
        round.wager_amount
    }

    fn assert_owner(env: &Env, address: &Address) {
        let owner: Address = env.storage().instance().get(&DataKey::Owner).unwrap();
        if *address != owner {
            panic_with_error!(env, Error::NotAuthorized);
        }
    }

    fn assert_admin(env: &Env, address: &Address) {
        let is_admin: bool = env
            .storage()
            .instance()
            .get(&DataKey::Admin(address.clone()))
            .unwrap_or(false);
        if !is_admin {
            panic_with_error!(env, Error::NotAuthorized);
        }
    }

    fn contains_string(v: &Vec<String>, target: &String) -> bool {
        for item in v.iter() {
            if item == *target {
                return true;
            }
        }
        false
    }

    /// Fisher-Yates shuffle over a small (4-element) options list, mirroring
    /// `shuffle_array` in the Cairo contract.
    fn shuffle_strings(env: &Env, arr: Vec<String>, seed: u64) -> Vec<String> {
        let mut result = arr;
        let mut current_seed = seed;
        let mut j = result.len();
        while j > 1 {
            j -= 1;
            current_seed =
                current_seed.wrapping_mul(1664525).wrapping_add(1013904223) % 0xFFFF_FFFFu64;
            let rand_idx = (current_seed % (j as u64 + 1)) as u32;
            if j != rand_idx {
                let a = result.get(j).unwrap();
                let b = result.get(rand_idx).unwrap();
                result.set(j, b);
                result.set(rand_idx, a);
            }
        }
        let _ = env;
        result
    }

    /// Ported from `_get_random_numbers` in the Cairo contract: hashes
    /// `(seed, ledger sequence, ledger timestamp, index)` in place of Cairo's
    /// `(seed, block_number, timestamp, index)` Poseidon-hash entropy, then
    /// reduces mod `limit` and dedupes until `amount` unique numbers are
    /// found. `for_index` mirrors the same +1 offset used when the numbers
    /// are card IDs rather than array indices.
    fn get_random_numbers(
        env: &Env,
        seed: u64,
        amount: u64,
        limit: u64,
        for_index: bool,
    ) -> Vec<u64> {
        if amount > limit {
            panic_with_error!(env, Error::AmountExceedsLimit);
        }
        if limit == 0 {
            panic_with_error!(env, Error::LimitMustBeGreaterThanZero);
        }

        let sequence = env.ledger().sequence() as u64;
        let timestamp = env.ledger().timestamp();

        let mut unique_numbers: Vec<u64> = Vec::new(env);
        let mut i: u64 = 0;
        while (unique_numbers.len() as u64) < amount {
            let mut buf = [0u8; 32];
            buf[0..8].copy_from_slice(&seed.to_be_bytes());
            buf[8..16].copy_from_slice(&sequence.to_be_bytes());
            buf[16..24].copy_from_slice(&timestamp.to_be_bytes());
            buf[24..32].copy_from_slice(&i.to_be_bytes());
            let bytes = Bytes::from_array(env, &buf);
            let hash = env.crypto().sha256(&bytes).to_array();

            let mut num_bytes = [0u8; 8];
            num_bytes.copy_from_slice(&hash[0..8]);
            let rand_u64 = u64::from_be_bytes(num_bytes);
            let mut rand = rand_u64 % limit;
            if !for_index {
                rand += 1;
            }

            let mut seen = false;
            for n in unique_numbers.iter() {
                if n == rand {
                    seen = true;
                    break;
                }
            }
            if !seen {
                unique_numbers.push_back(rand);
            }

            i += 1;
        }
        unique_numbers
    }
}
