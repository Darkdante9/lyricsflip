#![no_std]

mod errors;
mod events;
mod types;

#[cfg(test)]
mod test;

pub use errors::Error;
pub use events::{PlayerReady, RoundCompleted, RoundCreated, RoundJoined, RoundStarted};
pub use types::{Answer, Card, DataKey, Genre, PlayerStats, QuestionCard, QuestionKind, Role, Round};

use soroban_sdk::{
    contract, contractimpl, panic_with_error, Address, Bytes, Env, Map, String, Vec,
};

const DEFAULT_ROUND_DURATION_SECONDS: u64 = 300;

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

            let mut open = Self::read_open_rounds(&env);
            if let Some(idx) = open.first_index_of(round_id) {
                open.remove(idx);
                env.storage().persistent().set(&DataKey::OpenRounds, &open);
            }

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
        env.storage().persistent().set(
            &DataKey::RoundCardStartedAt((round_id, card_id)),
            &env.ledger().timestamp(),
        );

        round.next_card_index += 1;
        if round.next_card_index >= round_cards.len() {
            round.is_completed = true;
        }
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
        if round.is_completed {
            panic_with_error!(env, Error::RoundCompleted);
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

    pub fn build_question_card(env: Env, card: Card, seed: u64, kind: QuestionKind) -> QuestionCard {
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
        let random_ids = Self::get_random_numbers(env, seed, 10, cards_count, false);

        let mut false_answers: Vec<String> = Vec::new(env);
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
            let ids = Self::get_random_numbers(env, extra_seed, 1, cards_count, false);
            let id = ids.get(0).unwrap();
            let candidate = Self::get_card(env.clone(), id);
            if candidate.title != card.title
                && !Self::contains_string(&false_answers, &candidate.title)
            {
                false_answers.push_back(candidate.title.clone());
            }
            extra_seed += 1;
        }

        let mut options: Vec<String> = Vec::new(env);
        options.push_back(card.title.clone());
        for answer in false_answers.iter() {
            options.push_back(answer.clone());
        }

        let shuffled = Self::shuffle_strings(env, options, seed);

        QuestionCard {
            lyric: card.lyrics.clone(),
            timestamp: env.ledger().timestamp(),
            kind: QuestionKind::Title,
            option_one: shuffled.get(0).unwrap(),
            option_two: shuffled.get(1).unwrap(),
            option_three: shuffled.get(2).unwrap(),
            option_four: shuffled.get(3).unwrap(),
        }
    }

    fn build_artist_question(env: &Env, card: Card, seed: u64, cards_count: u64) -> QuestionCard {
        let random_ids = Self::get_random_numbers(env, seed, 10, cards_count, false);

        let mut false_answers: Vec<String> = Vec::new(env);
        for id in random_ids.iter() {
            if false_answers.len() >= 3 {
                break;
            }
            let candidate = Self::get_card(env.clone(), id);
            if candidate.artist != card.artist
                && !Self::contains_string(&false_answers, &candidate.artist)
            {
                false_answers.push_back(candidate.artist.clone());
            }
        }

        let mut extra_seed = seed + 1;
        while false_answers.len() < 3 {
            let ids = Self::get_random_numbers(env, extra_seed, 1, cards_count, false);
            let id = ids.get(0).unwrap();
            let candidate = Self::get_card(env.clone(), id);
            if candidate.artist != card.artist
                && !Self::contains_string(&false_answers, &candidate.artist)
            {
                false_answers.push_back(candidate.artist.clone());
            }
            extra_seed += 1;
        }

        let mut options: Vec<String> = Vec::new(env);
        options.push_back(card.artist.clone());
        for answer in false_answers.iter() {
            options.push_back(answer.clone());
        }

        let shuffled = Self::shuffle_strings(env, options, seed);

        QuestionCard {
            lyric: card.lyrics.clone(),
            timestamp: env.ledger().timestamp(),
            kind: QuestionKind::Artist,
            option_one: shuffled.get(0).unwrap(),
            option_two: shuffled.get(1).unwrap(),
            option_three: shuffled.get(2).unwrap(),
            option_four: shuffled.get(3).unwrap(),
        }
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
