#![no_std]

mod errors;
mod events;
mod types;

#[cfg(test)]
mod test;

pub use errors::Error;
pub use events::{PlayerReady, RoundCompleted, RoundCreated, RoundJoined, RoundStarted};
pub use types::{
    Answer, Card, DataKey, Genre, PlayerStats, QuestionCard, QuestionKind, Role, Round,
};

use soroban_sdk::{
    contract, contractimpl, panic_with_error, Address, Bytes, Env, Map, String, Vec,
};

const DEFAULT_ROUND_DURATION_SECONDS: u64 = 300;

/// Upper bound on the page size of the paginated list views (`get_rounds`,
/// `get_open_rounds`). Larger `limit` values are clamped to this.
pub const MAX_PAGE_LIMIT: u32 = 50;

/// Upper bound on the number of candidate cards inspected per fallback attempt
/// when building a question card. `build_question_card` never asks
/// `get_random_numbers` for more ids than `min(10, cards_count)`, so small
/// card sets no longer panic with `AmountExceedsLimit` (LF-010).
const MAX_DISTRACTOR_SAMPLE: u64 = 10;

// ---------------------------------------------------------------------------
// LF-012 – TTL policy
//
// Soroban persistent and instance entries are archived when their TTL expires.
// We extend TTLs on every write (and on reads for hot keys) so that active
// game data stays available on testnet / mainnet.
//
// Ledger cadence on Stellar mainnet ≈ 5 s, so:
//   DAY_IN_LEDGERS  ≈ 17 280 ledgers/day
//   BUMP_AMOUNT     = 30 days of ledgers
//   LIFETIME_THRESHOLD = 7 days — extend only when less than this remains,
//                        avoiding a per-call extend when lots of TTL is left.
// ---------------------------------------------------------------------------
pub const DAY_IN_LEDGERS: u32 = 17_280;
pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS; // ~30 days
pub const LIFETIME_THRESHOLD: u32 = 7 * DAY_IN_LEDGERS; // ~7 days

/// Extend instance storage TTL (owner, admin map, counters, config).
#[inline]
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Extend a single persistent storage entry by key.
#[inline]
fn bump_persistent<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(env: &Env, key: &K) {
    env.storage()
        .persistent()
        .extend_ttl(key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

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
        bump_instance(&env);
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
        if env
            .storage()
            .persistent()
            .get(&DataKey::RoundFinalized(round_id))
            .unwrap_or(false)
        {
            panic_with_error!(env, Error::RoundAlreadyFinalized);
        }

        let is_all_cards_answered = Self::are_all_required_answers_submitted(&env, round_id);
        let is_past_deadline = round.end_time != 0 && env.ledger().timestamp() >= round.end_time;
        if !is_all_cards_answered && !is_past_deadline {
            panic_with_error!(env, Error::RoundNotReady);
        }

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
                        .set(&DataKey::PlayerStats(player.clone()), &stats);
                    bump_persistent(&env, &DataKey::PlayerStats(player));
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
        bump_persistent(&env, &DataKey::RoundFinalized(round_id));
        bump_instance(&env);
    }

    pub fn get_cards_count(env: Env) -> u64 {
        bump_instance(&env);
        env.storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0)
    }

    pub fn get_round_count(env: Env) -> u64 {
        bump_instance(&env);
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

    pub fn get_cards_per_round(env: Env) -> u32 {
        bump_instance(&env);
        env.storage()
            .instance()
            .get(&DataKey::CardsPerRound)
            .unwrap_or(0)
    }

    pub fn get_card(env: Env, card_id: u64) -> Card {
        let card: Card = env
            .storage()
            .persistent()
            .get(&DataKey::Card(card_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::NonExistingCard));
        bump_persistent(&env, &DataKey::Card(card_id));
        card
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
        bump_persistent(&env, &DataKey::GenreCards(genre));
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
            .get(&DataKey::ArtistCards(artist.clone()))
            .unwrap_or(Vec::new(&env));
        let limit = ids.len() as u64;
        if limit == 0 {
            panic_with_error!(env, Error::ArtistCardsIsZero);
        }
        bump_persistent(&env, &DataKey::ArtistCards(artist));
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
        bump_persistent(&env, &DataKey::YearCards(year));
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
        let stats: PlayerStats = env
            .storage()
            .persistent()
            .get(&DataKey::PlayerStats(player.clone()))
            .unwrap_or(PlayerStats::zero());
        // Extend TTL on read so active players' stats don't get archived.
        if env
            .storage()
            .persistent()
            .has(&DataKey::PlayerStats(player.clone()))
        {
            bump_persistent(&env, &DataKey::PlayerStats(player));
        }
        stats
    }

    pub fn is_admin(env: Env, role: Role, address: Address) -> bool {
        let Role::Admin = role;
        bump_instance(&env);
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
        };

        let mut players: Vec<Address> = Vec::new(&env);
        players.push_back(caller.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RoundPlayers(round_id), &players);
        bump_persistent(&env, &DataKey::RoundPlayers(round_id));

        env.storage()
            .persistent()
            .set(&DataKey::RoundCards(round_id), &cards);
        bump_persistent(&env, &DataKey::RoundCards(round_id));

        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);
        bump_persistent(&env, &DataKey::Round(round_id));

        let mut open = Self::read_open_rounds(&env);
        open.push_back(round_id);
        env.storage().persistent().set(&DataKey::OpenRounds, &open);
        bump_persistent(&env, &DataKey::OpenRounds);

        bump_instance(&env);

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
        bump_persistent(&env, &ready_key);

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
        bump_persistent(&env, &ready_count_key);

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
                    .set(&DataKey::PlayerStats(player.clone()), &stats);
                bump_persistent(&env, &DataKey::PlayerStats(player));
            }

            let start_time = env.ledger().timestamp();
            round.start_time = start_time;
            round.end_time = start_time + DEFAULT_ROUND_DURATION_SECONDS;
            round.is_started = true;
            env.storage()
                .persistent()
                .set(&DataKey::Round(round_id), &round);
            bump_persistent(&env, &DataKey::Round(round_id));

            let mut open = Self::read_open_rounds(&env);
            if let Some(idx) = open.first_index_of(round_id) {
                open.remove(idx);
                env.storage().persistent().set(&DataKey::OpenRounds, &open);
                bump_persistent(&env, &DataKey::OpenRounds);
            }

            bump_instance(&env);

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
        if round.is_started {
            panic_with_error!(env, Error::RoundAlreadyStarted);
        }

        let mut players = Self::read_round_players(&env, round_id);
        players.push_back(caller.clone());
        env.storage()
            .persistent()
            .set(&DataKey::RoundPlayers(round_id), &players);
        bump_persistent(&env, &DataKey::RoundPlayers(round_id));

        RoundJoined {
            round_id,
            player: caller,
            joined_time: env.ledger().timestamp(),
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

        let started_at_key = DataKey::RoundCardStartedAt((round_id, card_id));
        env.storage()
            .persistent()
            .set(&started_at_key, &env.ledger().timestamp());
        bump_persistent(&env, &started_at_key);

        round.next_card_index += 1;
        if round.next_card_index >= round_cards.len() {
            round.is_completed = true;
        }
        env.storage()
            .persistent()
            .set(&DataKey::Round(round_id), &round);
        bump_persistent(&env, &DataKey::Round(round_id));

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
        bump_instance(&env);
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
        bump_persistent(&env, &DataKey::ArtistCards(card.artist.clone()));

        let mut genre_cards: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::GenreCards(card.genre))
            .unwrap_or(Vec::new(&env));
        genre_cards.push_back(card_id);
        env.storage()
            .persistent()
            .set(&DataKey::GenreCards(card.genre), &genre_cards);
        bump_persistent(&env, &DataKey::GenreCards(card.genre));

        let mut year_cards: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::YearCards(card.year))
            .unwrap_or(Vec::new(&env));
        year_cards.push_back(card_id);
        env.storage()
            .persistent()
            .set(&DataKey::YearCards(card.year), &year_cards);
        bump_persistent(&env, &DataKey::YearCards(card.year));

        env.storage()
            .persistent()
            .set(&DataKey::Card(card_id), &card);
        bump_persistent(&env, &DataKey::Card(card_id));

        env.storage().instance().set(&DataKey::CardsCount, &card_id);
        bump_instance(&env);
    }

    pub fn set_role(env: Env, caller: Address, recipient: Address, role: Role, is_enable: bool) {
        caller.require_auth();
        let owner: Address = env.storage().instance().get(&DataKey::Owner).unwrap();
        if caller != owner {
            panic_with_error!(env, Error::NotAuthorized);
        }
        Self::assert_admin(&env, &caller);
        let Role::Admin = role;
        env.storage()
            .instance()
            .set(&DataKey::Admin(recipient), &is_enable);
        bump_instance(&env);
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
        if env
            .storage()
            .persistent()
            .get(&DataKey::RoundFinalized(round_id))
            .unwrap_or(false)
        {
            panic_with_error!(env, Error::RoundAlreadyFinalized);
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
        bump_persistent(&env, &answered_key);

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
        bump_persistent(&env, &DataKey::RoundAnswerTimes(round_id));

        let is_answer_correct = match answer {
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
            bump_persistent(&env, &DataKey::RoundScores(round_id));
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
            .set(&DataKey::PlayerStats(caller.clone()), &stats);
        bump_persistent(&env, &DataKey::PlayerStats(caller));

        is_answer_correct
    }

    pub fn build_question_card(
        env: Env,
        card: Card,
        seed: u64,
        kind: QuestionKind,
    ) -> QuestionCard {
        let cards_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CardsCount)
            .unwrap_or(0);

        match kind {
            QuestionKind::Title => Self::build_title_question(&env, card, seed, cards_count),
            QuestionKind::Artist => Self::build_artist_question(&env, card, seed, cards_count),
            QuestionKind::Year => Self::build_year_question(&env, card, seed),
        }
    }

    // ---- build_question_card helpers ----

    fn build_title_question(env: &Env, card: Card, seed: u64, cards_count: u64) -> QuestionCard {
        let correct = card.title.clone();
        Self::build_options_question(
            env,
            card,
            seed,
            cards_count,
            QuestionKind::Title,
            correct,
            |candidate: &Card| candidate.title.clone(),
        )
    }

    fn build_artist_question(env: &Env, card: Card, seed: u64, cards_count: u64) -> QuestionCard {
        let correct = card.artist.clone();
        Self::build_options_question(
            env,
            card,
            seed,
            cards_count,
            QuestionKind::Artist,
            correct,
            |candidate: &Card| candidate.artist.clone(),
        )
    }

    /// Builds a multiple-choice card from the correct value plus three distinct
    /// distractor values drawn from the card catalogue.
    ///
    /// LF-010: instead of requesting a fixed 10 random ids (which panics with
    /// `AmountExceedsLimit` on small catalogues) and reseeding forever until 3
    /// distinct distractors show up (which burns the whole CPU budget when the
    /// catalogue has fewer than 4 distinct values), we walk the shuffled
    /// catalogue in bounded windows:
    ///
    /// * Each attempt inspects at most `min(10, cards_count)` candidates.
    /// * Same-genre cards are consulted first so the wrong options stay
    ///   plausible (they sound like the correct card).
    /// * Every card is examined at most once across all attempts, so the loop
    ///   always terminates; if fewer than 4 distinct values exist in the whole
    ///   catalogue the call fails with `NotEnoughDistinctCards` instead.
    fn build_options_question(
        env: &Env,
        card: Card,
        seed: u64,
        cards_count: u64,
        kind: QuestionKind,
        correct: String,
        get_value: impl Fn(&Card) -> String,
    ) -> QuestionCard {
        let candidates = Self::distractor_candidate_ids(env, &card, seed, cards_count);
        let total = candidates.len() as u64;
        let sample = core::cmp::min(MAX_DISTRACTOR_SAMPLE, total);
        let max_attempts = if sample == 0 {
            0
        } else {
            total.div_ceil(sample)
        };

        let mut false_answers: Vec<String> = Vec::new(env);
        let mut attempt: u64 = 0;
        while false_answers.len() < 3 && attempt < max_attempts {
            let start = attempt * sample;
            let mut seen: u64 = 0;
            while false_answers.len() < 3 && seen < sample {
                let id = candidates.get((start + seen) as u32).unwrap();
                let value = get_value(&Self::get_card(env.clone(), id));
                if value != correct && !Self::contains_string(&false_answers, &value) {
                    false_answers.push_back(value);
                }
                seen += 1;
            }
            attempt += 1;
        }
        if false_answers.len() < 3 {
            panic_with_error!(env, Error::NotEnoughDistinctCards);
        }

        let mut options: Vec<String> = Vec::new(env);
        options.push_back(correct);
        for answer in false_answers.iter() {
            options.push_back(answer.clone());
        }

        let shuffled = Self::shuffle_strings(env, options, seed);

        QuestionCard {
            lyric: card.lyrics.clone(),
            timestamp: env.ledger().timestamp(),
            kind,
            option_one: shuffled.get(0).unwrap(),
            option_two: shuffled.get(1).unwrap(),
            option_three: shuffled.get(2).unwrap(),
            option_four: shuffled.get(3).unwrap(),
        }
    }

    /// Ordered card ids used to pick distractors for `card`: same-genre cards
    /// (shuffled with `seed`) first so wrong options stay plausible, then every
    /// remaining card (shuffled with a derived seed). Each card appears exactly
    /// once, so a question can always be answered — or fail with
    /// `NotEnoughDistinctCards` — without ever looping.
    fn distractor_candidate_ids(env: &Env, card: &Card, seed: u64, cards_count: u64) -> Vec<u64> {
        let genre_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::GenreCards(card.genre))
            .unwrap_or(Vec::new(env));
        let genre_len = genre_ids.len() as u64;

        let mut candidates: Vec<u64> = Vec::new(env);

        if genre_len > 0 {
            let indices = Self::get_random_numbers(env, seed, genre_len, genre_len, true);
            for idx in indices.iter() {
                candidates.push_back(genre_ids.get(idx as u32).unwrap());
            }
        }

        let shuffled_all = Self::get_random_numbers(env, seed + 1, cards_count, cards_count, false);
        for id in shuffled_all.iter() {
            if !genre_ids.contains(&id) {
                candidates.push_back(id);
            }
        }
        candidates
    }

    /// Year distractors: pick 6 random offsets in the range [-5, +5] \ {0},
    /// deduplicate, and take the first 3. Year options are stored as their
    /// decimal string representation so they fit into the same `Vec<String>`
    /// shuffle as Title and Artist questions.
    fn build_year_question(env: &Env, card: Card, seed: u64) -> QuestionCard {
        // Generate offsets deterministically from the seed.
        let offsets: [i64; 10] = [1, -1, 2, -2, 3, -3, 4, -4, 5, -5];
        let mut current_seed = seed;
        // Fisher-Yates shuffle of the offsets array using the LCG.
        let mut shuffled_offsets = offsets;
        let mut j = 10usize;
        while j > 1 {
            j -= 1;
            current_seed =
                current_seed.wrapping_mul(1664525).wrapping_add(1013904223) % 0xFFFF_FFFFu64;
            let rand_idx = (current_seed % (j as u64 + 1)) as usize;
            shuffled_offsets.swap(j, rand_idx);
        }

        let correct_year = card.year as i64;
        let mut false_years: Vec<String> = Vec::new(env);
        for &offset in shuffled_offsets.iter() {
            if false_years.len() >= 3 {
                break;
            }
            let candidate_year = correct_year + offset;
            if candidate_year > 0 {
                let year_str = Self::u64_to_string(env, candidate_year as u64);
                if !Self::contains_string(&false_years, &year_str) {
                    false_years.push_back(year_str);
                }
            }
        }

        let correct_str = Self::u64_to_string(env, card.year);
        let mut options: Vec<String> = Vec::new(env);
        options.push_back(correct_str);
        for y in false_years.iter() {
            options.push_back(y.clone());
        }

        let shuffled = Self::shuffle_strings(env, options, seed);

        QuestionCard {
            lyric: card.lyrics.clone(),
            timestamp: env.ledger().timestamp(),
            kind: QuestionKind::Year,
            option_one: shuffled.get(0).unwrap(),
            option_two: shuffled.get(1).unwrap(),
            option_three: shuffled.get(2).unwrap(),
            option_four: shuffled.get(3).unwrap(),
        }
    }

    // ---- Internal helpers ----

    fn read_round(env: &Env, round_id: u64) -> Round {
        let round: Round = env
            .storage()
            .persistent()
            .get(&DataKey::Round(round_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::NonExistingRound));
        bump_persistent(env, &DataKey::Round(round_id));
        round
    }

    fn read_round_cards(env: &Env, round_id: u64) -> Vec<u64> {
        let cards = env
            .storage()
            .persistent()
            .get(&DataKey::RoundCards(round_id))
            .unwrap_or(Vec::new(env));
        bump_persistent(env, &DataKey::RoundCards(round_id));
        cards
    }

    fn read_round_players(env: &Env, round_id: u64) -> Vec<Address> {
        let players = env
            .storage()
            .persistent()
            .get(&DataKey::RoundPlayers(round_id))
            .unwrap_or(Vec::new(env));
        bump_persistent(env, &DataKey::RoundPlayers(round_id));
        players
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

    /// Converts a `u64` year value to its decimal string representation.
    /// Soroban's `no_std` environment has no format!/write! macros, so we
    /// build the string manually by repeated division.
    fn u64_to_string(env: &Env, mut n: u64) -> String {
        if n == 0 {
            return String::from_str(env, "0");
        }
        // Collect digits in reverse.
        let mut digits: [u8; 20] = [0u8; 20];
        let mut len = 0usize;
        while n > 0 {
            digits[len] = b'0' + (n % 10) as u8;
            n /= 10;
            len += 1;
        }
        // Reverse into a fixed-size array and build a `&str`.
        let mut buf: [u8; 20] = [0u8; 20];
        for i in 0..len {
            buf[i] = digits[len - 1 - i];
        }
        // SAFETY: all bytes are ASCII digits.
        let s = core::str::from_utf8(&buf[..len]).unwrap_or("0");
        String::from_str(env, s)
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

    /// LF-011 fix: partial Fisher-Yates shuffle — O(limit) with no
    /// deduplication loop.
    ///
    /// Builds an index array `[0, 1, …, limit-1]`, performs a single-pass
    /// Fisher-Yates shuffle seeded from `(seed, ledger_sequence,
    /// ledger_timestamp)`, and returns the first `amount` elements. This
    /// replaces the old SHA-256 + dedup loop which had coupon-collector
    /// worst-case cost when `amount ≈ limit`.
    ///
    /// `for_index`: when `false` the results are shifted by +1 so they
    /// become 1-based card IDs instead of 0-based array indices (mirrors the
    /// same offset used by the old implementation).
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

        // Build a compact index array [0..limit).
        let mut indices: Vec<u64> = Vec::new(env);
        for i in 0..limit {
            indices.push_back(i);
        }

        // Seed an LCG from the on-chain entropy.
        // Using the same 32-byte SHA-256 block as before, but only once.
        let sequence = env.ledger().sequence() as u64;
        let timestamp = env.ledger().timestamp();
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&seed.to_be_bytes());
        buf[8..16].copy_from_slice(&sequence.to_be_bytes());
        buf[16..24].copy_from_slice(&timestamp.to_be_bytes());
        // last 8 bytes left zero — distinguishes this from per-iteration hashes
        let bytes = Bytes::from_array(env, &buf);
        let hash = env.crypto().sha256(&bytes).to_array();
        let mut rng_state = u64::from_be_bytes([
            hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
        ]);

        // Partial Fisher-Yates: shuffle only the first `amount` positions.
        // Iteration i swaps indices[i] with a random position in [i, limit).
        let mut i: u32 = 0;
        while (i as u64) < amount {
            // LCG step (Numerical Recipes constants, same as shuffle_strings).
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let range = limit - (i as u64);
            let j = (i as u64) + (rng_state % range);
            // swap indices[i] and indices[j]
            let a = indices.get(i).unwrap();
            let b = indices.get(j as u32).unwrap();
            indices.set(i, b);
            indices.set(j as u32, a);
            i += 1;
        }

        // Collect the first `amount` shuffled indices, applying the +1 offset
        // when the caller wants 1-based card IDs.
        let mut result: Vec<u64> = Vec::new(env);
        for k in 0..(amount as u32) {
            let v = indices.get(k).unwrap();
            result.push_back(if for_index { v } else { v + 1 });
        }
        result
    }
}
