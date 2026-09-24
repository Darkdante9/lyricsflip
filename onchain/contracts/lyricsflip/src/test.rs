//! Representative behavior-coverage tests ported from
//! `onchain/src/tests/test_lyricsflip.cairo` (round lifecycle, card queries,
//! answer submission, and access control). These are written against the
//! soroban-sdk 22 testutils API but have not been `cargo test`-verified in
//! this environment (no Rust toolchain available) — run `cargo test` inside
//! `onchain/` before relying on them.

extern crate std;

use crate::{
    Answer, AnswerSubmitted, Card, CardAdded, CardDrawn, CardRemoved, CardUpdated,
    CardsPerRoundUpdated, Error, Genre, LyricsFlip, LyricsFlipClient, Role, RoleUpdated,
    RoundCompleted, MAX_CARDS_PER_BATCH, MAX_LYRICS_LEN, MAX_PAGE_LIMIT, MAX_ROUND_PLAYERS,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    Address, Env, Event, Map, String, Vec,
};

/// 2026-05-28, so cards up to year 2026 pass validation.
const NOW: u64 = 1_780_000_000;

fn setup<'a>() -> (Env, LyricsFlipClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    let owner = Address::generate(&env);
    let contract_id = env.register(LyricsFlip, (owner.clone(),));
    let client = LyricsFlipClient::new(&env, &contract_id);
    (env, client, owner)
}

fn sample_card(env: &Env, genre: Genre, artist: &str, title: &str, year: u64) -> Card {
    Card {
        card_id: 0,
        genre,
        artist: String::from_str(env, artist),
        title: String::from_str(env, title),
        year,
        lyrics: String::from_str(env, "sample lyric line"),
    }
}

#[test]
fn constructor_grants_owner_admin() {
    let (_env, client, owner) = setup();
    assert!(client.is_admin(&Role::Admin, &owner));
}

#[test]
fn add_card_requires_admin() {
    let (env, client, _owner) = setup();
    let non_admin = Address::generate(&env);
    let card = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    let result = client.try_add_card(&non_admin, &card);
    assert!(result.is_err());
}

#[test]
fn add_card_and_get_card_round_trip() {
    let (env, client, owner) = setup();
    let card = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    client.add_card(&owner, &card);

    let stored = client.get_card(&1);
    assert_eq!(stored.title, card.title);
    assert_eq!(stored.artist, card.artist);
    assert_eq!(stored.year, 1999);
}

#[test]
fn create_round_requires_a_genre() {
    let (env, client, owner) = setup();
    let _ = env;
    let result = client.try_create_round(&owner, &None, &1u64);
    assert!(result.is_err());
}

fn seed_cards(env: &Env, client: &LyricsFlipClient, owner: &Address, count: u64) {
    for i in 0..count {
        let title = std::format!("Title A{}", i);
        let card = sample_card(env, Genre::Pop, "Artist A", &title, 2000 + i);
        client.add_card(owner, &card);
    }
}

#[test]
fn round_requires_all_players_ready_before_starting() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &42u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);

    client.start_round(&owner, &round_id);
    let round = client.get_round(&round_id);
    assert!(
        !round.is_started,
        "round should not auto-start until every player is ready"
    );

    client.start_round(&player2, &round_id);
    let round = client.get_round(&round_id);
    assert!(
        round.is_started,
        "round should start once all players signal ready"
    );
}

#[test]
fn total_rounds_increments_once_per_player_when_three_player_round_starts() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);

    let player2 = Address::generate(&env);
    let player3 = Address::generate(&env);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &42u64);
    client.join_round(&player2, &round_id);
    client.join_round(&player3, &round_id);

    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);
    client.start_round(&player3, &round_id);

    let round = client.get_round(&round_id);
    assert!(
        round.is_started,
        "round should start once all players are ready"
    );

    assert_eq!(client.get_player_stat(&owner).total_rounds, 1);
    assert_eq!(client.get_player_stat(&player2).total_rounds, 1);
    assert_eq!(client.get_player_stat(&player3).total_rounds, 1);
}

#[test]
fn total_rounds_unchanged_when_round_never_fully_starts() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);

    let player2 = Address::generate(&env);
    let player3 = Address::generate(&env);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &42u64);
    client.join_round(&player2, &round_id);
    client.join_round(&player3, &round_id);

    // Only some players mark ready; the round never reaches full ready count.
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let round = client.get_round(&round_id);
    assert!(
        !round.is_started,
        "round should not start until every player is ready"
    );

    assert_eq!(client.get_player_stat(&owner).total_rounds, 0);
    assert_eq!(client.get_player_stat(&player2).total_rounds, 0);
    assert_eq!(client.get_player_stat(&player3).total_rounds, 0);
}

#[test]
fn join_round_rejects_duplicate_join() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &1u64);

    let result = client.try_join_round(&owner, &round_id);
    assert!(result.is_err(), "the round creator is already a player");
}

#[test]
fn join_round_rejects_after_start() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &1u64);
    client.start_round(&owner, &round_id);

    let late_player = Address::generate(&env);
    let result = client.try_join_round(&late_player, &round_id);
    assert!(result.is_err());
}

#[test]
fn submit_answer_updates_streak_and_reports_correctness() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &3);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &7u64);
    client.start_round(&owner, &round_id);

    let round = client.get_round(&round_id);
    assert!(round.is_started);

    let card = client.next_card(&round_id);

    let correct = client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone()));
    assert!(correct);

    let stats = client.get_player_stat(&owner);
    assert_eq!(stats.current_streak, 1);
    assert_eq!(stats.max_streak, 1);

    let wrong = client.submit_answer(
        &owner,
        &round_id,
        &Answer::Title(String::from_str(&env, "definitely not the title")),
    );
    assert!(!wrong);
    let stats = client.get_player_stat(&owner);
    assert_eq!(stats.current_streak, 0);
    assert_eq!(stats.max_streak, 1);
}

#[test]
fn get_cards_of_genre_returns_requested_amount() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 6);
    client.set_cards_per_round(&owner, &4);

    let cards = client.get_cards_of_genre(&Genre::Pop, &99u64);
    assert_eq!(cards.len(), 4);
}

#[test]
fn get_cards_of_a_year_errors_when_empty() {
    let (_env, client, _owner) = setup();
    let result = client.try_get_cards_of_a_year(&1975u64, &1u64);
    assert!(result.is_err());
}

#[test]
fn set_role_is_owner_gated_and_updates_is_admin() {
    let (env, client, owner) = setup();
    let new_admin = Address::generate(&env);
    assert!(!client.is_admin(&Role::Admin, &new_admin));

    client.set_role(&owner, &new_admin, &Role::Admin, &true);
    assert!(client.is_admin(&Role::Admin, &new_admin));

    let outsider = Address::generate(&env);
    let result = client.try_set_role(&outsider, &new_admin, &Role::Admin, &false);
    assert!(result.is_err(), "only the owner may grant/revoke admin");
}

fn assert_emitted(env: &Env, contract_id: &Address, event: impl Event) {
    let expected = event.to_xdr(env, contract_id);
    assert!(
        env.events().all().events().contains(&expected),
        "expected event {:?} was not emitted",
        expected
    );
}

fn assert_round_completed_event(
    env: &Env,
    contract_id: &Address,
    round_id: u64,
    winners: &soroban_sdk::Vec<Address>,
    scores: &Map<Address, u64>,
) {
    assert_emitted(
        env,
        contract_id,
        RoundCompleted {
            round_id,
            winners: winners.clone(),
            scores: scores.clone(),
        },
    );
}

#[test]
fn finalize_round_e2e_single_winner_updates_stats_and_scores() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &2);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &42u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let first_card = client.next_card(&round_id);
    env.ledger().set_timestamp(100);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(first_card.title.clone())));
    env.ledger().set_timestamp(130);
    assert!(!client.submit_answer(
        &player2,
        &round_id,
        &Answer::Title(String::from_str(&env, "wrong-a"))
    ));

    let second_card = client.next_card(&round_id);
    env.ledger().set_timestamp(200);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(second_card.title.clone())));
    env.ledger().set_timestamp(240);
    assert!(!client.submit_answer(
        &player2,
        &round_id,
        &Answer::Title(String::from_str(&env, "wrong-b"))
    ));

    let scores = client.get_round_scores(&round_id);
    assert_eq!(scores.get(owner.clone()).unwrap(), 2u64);
    assert_eq!(scores.get(player2.clone()).unwrap(), 0u64);

    client.finalize_round(&owner, &round_id);

    let owner_stats = client.get_player_stat(&owner);
    assert_eq!(owner_stats.rounds_won, 1);
    assert_eq!(client.get_player_stat(&player2).rounds_won, 0);

    let winners = soroban_sdk::vec![&env, owner.clone()];
    assert_round_completed_event(&env, &client.address, round_id, &winners, &scores);

    let event_count_before_second_finalize = env
        .events()
        .all()
        .filter_by_contract(&client.address)
        .events()
        .len();
    let second_attempt = client.try_finalize_round(&owner, &round_id);
    assert!(second_attempt.is_err());
    let event_count_after_second_finalize = env
        .events()
        .all()
        .filter_by_contract(&client.address)
        .events()
        .len();
    assert_eq!(
        event_count_after_second_finalize,
        event_count_before_second_finalize
    );
}

#[test]
fn finalize_round_e2e_deadline_path_works_without_all_answers() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &2);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &99u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let card = client.next_card(&round_id);
    env.ledger().set_timestamp(50);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone())));
    env.ledger().set_timestamp(70);
    assert!(!client.submit_answer(
        &player2,
        &round_id,
        &Answer::Title(String::from_str(&env, "not-the-answer"))
    ));

    let round = client.get_round(&round_id);
    env.ledger().set_timestamp(round.end_time + 1);

    client.finalize_round(&owner, &round_id);

    let stats = client.get_player_stat(&owner);
    assert_eq!(stats.rounds_won, 1);
    let winners = soroban_sdk::vec![&env, owner.clone()];
    assert_round_completed_event(
        &env,
        &client.address,
        round_id,
        &winners,
        &client.get_round_scores(&round_id),
    );
}

#[test]
fn finalize_round_e2e_tie_breaks_by_total_answer_time() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &1);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &3u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let card = client.next_card(&round_id);
    env.ledger().set_timestamp(100);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone())));
    env.ledger().set_timestamp(130);
    assert!(client.submit_answer(&player2, &round_id, &Answer::Title(card.title.clone())));

    client.finalize_round(&owner, &round_id);

    assert_eq!(client.get_player_stat(&owner).rounds_won, 1);
    assert_eq!(client.get_player_stat(&player2).rounds_won, 0);
}

#[test]
fn finalize_round_e2e_exact_score_and_time_tie_produces_co_winners() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &1);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &11u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let card = client.next_card(&round_id);
    env.ledger().set_timestamp(100);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone())));
    assert!(client.submit_answer(&player2, &round_id, &Answer::Title(card.title.clone())));

    client.finalize_round(&owner, &round_id);

    assert_eq!(client.get_player_stat(&owner).rounds_won, 1);
    assert_eq!(client.get_player_stat(&player2).rounds_won, 1);

    let winners = soroban_sdk::vec![&env, owner.clone(), player2.clone()];
    let scores = client.get_round_scores(&round_id);
    assert_round_completed_event(&env, &client.address, round_id, &winners, &scores);
}

#[test]
fn finalize_round_rejects_early_and_non_participant() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &1);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &17u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let card = client.next_card(&round_id);
    env.ledger().set_timestamp(100);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone())));

    let err = client.try_finalize_round(&owner, &round_id);
    assert!(err.is_err());

    let outsider = Address::generate(&env);
    let outsider_err = client.try_finalize_round(&outsider, &round_id);
    assert!(outsider_err.is_err());
}

#[test]
fn finalize_round_e2e_no_correct_answers_has_no_winners() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &1);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &17u64);
    let player2 = Address::generate(&env);
    client.join_round(&player2, &round_id);
    client.start_round(&owner, &round_id);
    client.start_round(&player2, &round_id);

    let card = client.next_card(&round_id);
    env.ledger().set_timestamp(100);
    assert!(!client.submit_answer(
        &owner,
        &round_id,
        &Answer::Title(String::from_str(&env, "wrong"))
    ));
    env.ledger().set_timestamp(150);
    assert!(!client.submit_answer(
        &player2,
        &round_id,
        &Answer::Title(String::from_str(&env, "wrong-two"))
    ));

    client.finalize_round(&owner, &round_id);

    assert_eq!(client.get_player_stat(&owner).rounds_won, 0);
    assert_eq!(client.get_player_stat(&player2).rounds_won, 0);

    let scores = client.get_round_scores(&round_id);
    assert_eq!(scores.get(owner.clone()).unwrap(), 0u64);
    assert_eq!(scores.get(player2.clone()).unwrap(), 0u64);

    let winners = soroban_sdk::vec![&env];
    assert_round_completed_event(&env, &client.address, round_id, &winners, &scores);
}

#[test]
fn card_counts_track_adds() {
    let (env, client, owner) = setup();
    assert_eq!(client.get_cards_count(), 0);
    assert_eq!(client.get_genre_card_count(&Genre::Pop), 0);

    seed_cards(&env, &client, &owner, 3);
    client.add_card(
        &owner,
        &sample_card(&env, Genre::Rock, "Artist B", "Title B", 1990),
    );

    assert_eq!(client.get_cards_count(), 4);
    assert_eq!(client.get_genre_card_count(&Genre::Pop), 3);
    assert_eq!(client.get_genre_card_count(&Genre::Rock), 1);
    assert_eq!(client.get_genre_card_count(&Genre::Jazz), 0);
}

#[test]
fn round_count_tracks_creates() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);
    assert_eq!(client.get_round_count(), 0);

    for seed in 0..3u64 {
        client.create_round(&owner, &Some(Genre::Pop), &seed);
    }
    assert_eq!(client.get_round_count(), 3);
}

#[test]
fn get_rounds_paginates_and_respects_limit() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);
    for seed in 0..5u64 {
        client.create_round(&owner, &Some(Genre::Pop), &seed);
    }

    let page = client.get_rounds(&1, &2);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0).unwrap().round_id, 1);
    assert_eq!(page.get(1).unwrap().round_id, 2);

    let page = client.get_rounds(&4, &10);
    assert_eq!(page.len(), 2, "only rounds 4 and 5 remain");
    assert_eq!(page.get(0).unwrap().round_id, 4);

    // Round ids start at 1, so start = 0 behaves like start = 1.
    assert_eq!(client.get_rounds(&0, &1).get(0).unwrap().round_id, 1);
    assert_eq!(client.get_rounds(&6, &10).len(), 0);
    assert_eq!(client.get_rounds(&1, &0).len(), 0);
}

#[test]
fn get_rounds_clamps_limit_to_max() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &1);
    let total = MAX_PAGE_LIMIT as u64 + 5;
    for seed in 0..total {
        client.create_round(&owner, &Some(Genre::Pop), &seed);
    }

    let page = client.get_rounds(&1, &u32::MAX);
    assert_eq!(page.len(), MAX_PAGE_LIMIT);
    assert_eq!(client.get_open_rounds(&0, &u32::MAX).len(), MAX_PAGE_LIMIT);
}

#[test]
fn open_rounds_drop_rounds_once_started() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);
    let r1 = client.create_round(&owner, &Some(Genre::Pop), &1u64);
    let r2 = client.create_round(&owner, &Some(Genre::Pop), &2u64);
    let r3 = client.create_round(&owner, &Some(Genre::Pop), &3u64);

    let open = client.get_open_rounds(&0, &10);
    assert_eq!(open.len(), 3);
    assert_eq!(open.get(0).unwrap(), r1);

    // A round with a second player stays open until everyone is ready.
    let player = Address::generate(&env);
    client.join_round(&player, &r2);
    client.start_round(&owner, &r2);
    assert_eq!(client.get_open_rounds(&0, &10).len(), 3);
    client.start_round(&player, &r2);

    client.start_round(&owner, &r1);

    let open = client.get_open_rounds(&0, &10);
    assert_eq!(open.len(), 1);
    assert_eq!(open.get(0).unwrap(), r3);

    // Offset pagination.
    assert_eq!(client.get_open_rounds(&1, &10).len(), 0);
    assert_eq!(client.get_open_rounds(&5, &10).len(), 0);
}

/// Clients map on these numeric codes; renumbering any of them is a breaking
/// change. Update `onchain/README.md` and `frontend/src/lib/stellar/errors.ts`
/// together with this test.
#[test]
fn error_codes_are_stable() {
    let expected = [
        (Error::AlreadyInitialized, 1),
        (Error::NonExistingRound, 2),
        (Error::RoundAlreadyStarted, 3),
        (Error::NonExistingGenre, 4),
        (Error::RoundAlreadyJoined, 5),
        (Error::InvalidCardsPerRound, 6),
        (Error::ArtistCardsIsZero, 7),
        (Error::EmptyYearCards, 8),
        (Error::EmptyGenreCards, 9),
        (Error::RoundNotStarted, 10),
        (Error::RoundCompleted, 11),
        (Error::NotAParticipant, 12),
        (Error::AlreadyReady, 13),
        (Error::NotAuthorized, 14),
        (Error::AmountExceedsLimit, 15),
        (Error::LimitMustBeGreaterThanZero, 16),
        (Error::NonExistingCard, 17),
        (Error::RoundFull, 21),
        (Error::InvalidCardTitle, 26),
        (Error::InvalidCardArtist, 27),
        (Error::InvalidCardLyrics, 28),
        (Error::InvalidCardYear, 29),
        (Error::LyricsTooLong, 30),
        (Error::DuplicateCard, 31),
        (Error::BatchTooLarge, 32),
    ];
    for (variant, code) in expected {
        assert_eq!(variant as u32, code, "{:?} was renumbered", variant);
    }
}

fn card_n(env: &Env, genre: Genre, n: u64) -> Card {
    let artist = std::format!("Artist {}", n % 3);
    let title = std::format!("Title {}", n);
    sample_card(env, genre, &artist, &title, 1990 + n % 5)
}

fn genre_ids(client: &LyricsFlipClient, owner: &Address, genre: Genre) -> std::vec::Vec<u64> {
    // Draw every card of the genre and collect their ids, sorted.
    let count = client.get_genre_card_count(&genre);
    client.set_cards_per_round(owner, &count);
    let mut ids: std::vec::Vec<u64> = client
        .get_cards_of_genre(&genre, &1)
        .iter()
        .map(|c| c.card_id)
        .collect();
    ids.sort();
    ids
}

fn sorted_ids(cards: Vec<Card>) -> std::vec::Vec<u64> {
    let mut ids: std::vec::Vec<u64> = cards.iter().map(|c| c.card_id).collect();
    ids.sort();
    ids
}

#[test]
fn add_card_returns_id_and_stores_it() {
    let (env, client, owner) = setup();
    let mut card = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    card.card_id = 42;
    assert_eq!(client.add_card(&owner, &card), 1);
    assert_emitted(
        &env,
        &client.address,
        CardAdded {
            card_id: 1,
            genre: Genre::Pop,
        },
    );
    assert_eq!(client.get_card(&1).card_id, 1);
}

#[test]
fn add_card_rejects_invalid_cards() {
    let (env, client, owner) = setup();
    let ok = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    let empty = String::from_str(&env, "");

    let cases = [
        (
            Card {
                title: empty.clone(),
                ..ok.clone()
            },
            Error::InvalidCardTitle,
        ),
        (
            Card {
                artist: empty.clone(),
                ..ok.clone()
            },
            Error::InvalidCardArtist,
        ),
        (
            Card {
                lyrics: empty.clone(),
                ..ok.clone()
            },
            Error::InvalidCardLyrics,
        ),
        (
            Card {
                year: 0,
                ..ok.clone()
            },
            Error::InvalidCardYear,
        ),
        (
            Card {
                year: 1899,
                ..ok.clone()
            },
            Error::InvalidCardYear,
        ),
        (
            Card {
                year: 2027,
                ..ok.clone()
            },
            Error::InvalidCardYear,
        ),
        (
            Card {
                lyrics: String::from_str(&env, &"a".repeat(MAX_LYRICS_LEN as usize + 1)),
                ..ok.clone()
            },
            Error::LyricsTooLong,
        ),
    ];
    for (card, err) in cases {
        assert_eq!(client.try_add_card(&owner, &card), Err(Ok(err.into())));
    }
    assert_eq!(client.get_cards_count(), 0);

    // Boundaries are accepted.
    client.add_card(
        &owner,
        &Card {
            year: 1900,
            ..ok.clone()
        },
    );
    let lyrics = String::from_str(&env, &"a".repeat(MAX_LYRICS_LEN as usize));
    let edge = sample_card(&env, Genre::Pop, "Artist B", "Title B", 2026);
    client.add_card(&owner, &Card { lyrics, ..edge });
}

#[test]
fn add_card_rejects_duplicates() {
    let (env, client, owner) = setup();
    let card = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    client.add_card(&owner, &card);

    // Same title + artist, even with other fields changed.
    let dup = Card {
        genre: Genre::Rock,
        year: 2001,
        ..card.clone()
    };
    assert_eq!(
        client.try_add_card(&owner, &dup),
        Err(Ok(Error::DuplicateCard.into()))
    );

    // Same title by a different artist is fine.
    let other = sample_card(&env, Genre::Pop, "Artist B", "Title A", 1999);
    assert_eq!(client.add_card(&owner, &other), 2);
}

#[test]
fn update_card_moves_indexes() {
    let (env, client, owner) = setup();
    let a = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999),
    );
    let b = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title B", 1999),
    );
    let c = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title C", 1999),
    );

    let updated = sample_card(&env, Genre::Rock, "Artist Z", "Title A2", 2005);
    client.update_card(&owner, &a, &updated);
    assert_emitted(
        &env,
        &client.address,
        CardUpdated {
            card_id: a,
            genre: Genre::Rock,
        },
    );

    let stored = client.get_card(&a);
    assert_eq!(
        stored,
        Card {
            card_id: a,
            ..updated.clone()
        }
    );
    assert_eq!(client.get_cards_count(), 3);

    assert_eq!(genre_ids(&client, &owner, Genre::Pop), std::vec![b, c]);
    assert_eq!(genre_ids(&client, &owner, Genre::Rock), std::vec![a]);

    client.set_cards_per_round(&owner, &1);
    let artist_z = String::from_str(&env, "Artist Z");
    assert_eq!(
        sorted_ids(client.get_cards_of_artist(&artist_z, &1)),
        std::vec![a]
    );
    assert_eq!(
        sorted_ids(client.get_cards_of_a_year(&2005, &1)),
        std::vec![a]
    );

    client.set_cards_per_round(&owner, &2);
    let artist_a = String::from_str(&env, "Artist A");
    assert_eq!(
        sorted_ids(client.get_cards_of_artist(&artist_a, &1)),
        std::vec![b, c]
    );
    assert_eq!(
        sorted_ids(client.get_cards_of_a_year(&1999, &1)),
        std::vec![b, c]
    );

    // Title + artist of another card is a duplicate; keeping its own is not.
    let clash = sample_card(&env, Genre::Pop, "Artist A", "Title B", 1999);
    assert_eq!(
        client.try_update_card(&owner, &a, &clash),
        Err(Ok(Error::DuplicateCard.into()))
    );
    client.update_card(
        &owner,
        &a,
        &Card {
            year: 2006,
            ..updated.clone()
        },
    );
    assert_eq!(
        client.try_update_card(&owner, &99, &updated),
        Err(Ok(Error::NonExistingCard.into()))
    );

    // The old title + artist is free again.
    client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999),
    );
}

#[test]
fn remove_card_keeps_indexes_consistent() {
    let (env, client, owner) = setup();
    let a = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999),
    );
    let b = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title B", 1999),
    );
    let c = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title C", 1999),
    );

    client.remove_card(&owner, &a);
    assert_emitted(&env, &client.address, CardRemoved { card_id: a });

    assert_eq!(
        client.try_get_card(&a),
        Err(Ok(Error::NonExistingCard.into()))
    );
    assert_eq!(client.get_cards_count(), 2);
    assert_eq!(genre_ids(&client, &owner, Genre::Pop), std::vec![b, c]);
    let artist_a = String::from_str(&env, "Artist A");
    assert_eq!(
        sorted_ids(client.get_cards_of_artist(&artist_a, &1)),
        std::vec![b, c]
    );
    assert_eq!(
        sorted_ids(client.get_cards_of_a_year(&1999, &1)),
        std::vec![b, c]
    );

    // The moved card (c) can itself be removed cleanly.
    client.remove_card(&owner, &c);
    client.set_cards_per_round(&owner, &1);
    assert_eq!(genre_ids(&client, &owner, Genre::Pop), std::vec![b]);
    assert_eq!(
        sorted_ids(client.get_cards_of_a_year(&1999, &1)),
        std::vec![b]
    );

    client.remove_card(&owner, &b);
    assert_eq!(client.get_cards_count(), 0);
    assert_eq!(client.get_genre_card_count(&Genre::Pop), 0);
    assert_eq!(
        client.try_get_cards_of_artist(&artist_a, &1),
        Err(Ok(Error::ArtistCardsIsZero.into()))
    );
    assert_eq!(
        client.try_remove_card(&owner, &b),
        Err(Ok(Error::NonExistingCard.into()))
    );

    // Ids are not reused.
    let d = client.add_card(
        &owner,
        &sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999),
    );
    assert_eq!(d, 4);
}

#[test]
fn update_and_remove_card_require_admin() {
    let (env, client, owner) = setup();
    let card = sample_card(&env, Genre::Pop, "Artist A", "Title A", 1999);
    let id = client.add_card(&owner, &card);
    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_update_card(&stranger, &id, &card),
        Err(Ok(Error::NotAuthorized.into()))
    );
    assert_eq!(
        client.try_remove_card(&stranger, &id),
        Err(Ok(Error::NotAuthorized.into()))
    );
    let mut cards = Vec::new(&env);
    cards.push_back(card);
    assert_eq!(
        client.try_add_cards(&stranger, &cards),
        Err(Ok(Error::NotAuthorized.into()))
    );
}

#[test]
fn add_cards_batch_of_20_returns_sequential_ids() {
    let (env, client, owner) = setup();
    client.add_card(&owner, &card_n(&env, Genre::Rock, 100));

    let mut cards = Vec::new(&env);
    for n in 0..20 {
        cards.push_back(card_n(&env, Genre::Pop, n));
    }
    let ids = client.add_cards(&owner, &cards);
    assert_eq!(ids.len(), 20);
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(id, i as u64 + 2);
    }

    assert_eq!(client.get_cards_count(), 21);
    assert_eq!(client.get_genre_card_count(&Genre::Pop), 20);
    assert_eq!(
        genre_ids(&client, &owner, Genre::Pop),
        (2..=21).collect::<std::vec::Vec<u64>>()
    );

    client.set_cards_per_round(&owner, &7);
    let artist0 = String::from_str(&env, "Artist 0");
    assert_eq!(client.get_cards_of_artist(&artist0, &1).len(), 7);
    client.set_cards_per_round(&owner, &4);
    assert_eq!(client.get_cards_of_a_year(&1990, &1).len(), 4);
}

#[test]
fn add_cards_is_atomic_and_bounded() {
    let (env, client, owner) = setup();
    let mut cards = Vec::new(&env);
    cards.push_back(card_n(&env, Genre::Pop, 0));
    cards.push_back(card_n(&env, Genre::Pop, 0));
    assert_eq!(
        client.try_add_cards(&owner, &cards),
        Err(Ok(Error::DuplicateCard.into()))
    );
    assert_eq!(client.get_cards_count(), 0);

    let mut cards = Vec::new(&env);
    for n in 0..=MAX_CARDS_PER_BATCH as u64 {
        cards.push_back(card_n(&env, Genre::Pop, n));
    }
    assert_eq!(
        client.try_add_cards(&owner, &cards),
        Err(Ok(Error::BatchTooLarge.into()))
    );
}

/// Backs the `MAX_CARDS_PER_BATCH` figure documented in `onchain/README.md`:
/// a full batch stays within Soroban's per-transaction limits.
#[test]
fn add_cards_max_batch_fits_budget() {
    let (env, client, owner) = setup();
    let mut cards = Vec::new(&env);
    for n in 0..MAX_CARDS_PER_BATCH as u64 {
        cards.push_back(card_n(&env, Genre::Pop, n));
    }
    env.cost_estimate().budget().reset_default();
    client.add_cards(&owner, &cards);
    // Mainnet limits are enforced by default in tests, so reaching this point
    // already proves the batch fits; the asserts document the key figures.
    let res = env.cost_estimate().resources();
    std::println!("add_cards({}) resources: {:?}", MAX_CARDS_PER_BATCH, res);
    assert!(res.instructions <= 400_000_000);
    assert!(res.write_entries <= 200);
}

/// Adding card #1,000 to a genre costs about the same as adding card #1,
/// since indexes are stored per item instead of as one growing vector: the
/// ledger entries and bytes it reads and writes (what the network meters and
/// charges for) stay constant. Instruction counts are not compared because
/// the test host keeps the whole ledger in one in-memory map, so every write
/// gets slower as the test ledger grows regardless of the contract.
#[test]
fn add_card_cost_does_not_grow_with_index_size() {
    let (env, client, owner) = setup();
    env.cost_estimate().disable_resource_limits();
    env.cost_estimate().budget().reset_default();
    client.add_card(&owner, &card_n(&env, Genre::Pop, 0));
    let first = env.cost_estimate().resources();

    env.cost_estimate().budget().reset_unlimited();
    let mut cards = Vec::new(&env);
    for n in 1..999u64 {
        cards.push_back(card_n(&env, Genre::Pop, n));
        if cards.len() == MAX_CARDS_PER_BATCH {
            client.add_cards(&owner, &cards);
            cards = Vec::new(&env);
        }
    }
    if !cards.is_empty() {
        client.add_cards(&owner, &cards);
    }
    assert_eq!(client.get_genre_card_count(&Genre::Pop), 999);

    env.cost_estimate().budget().reset_default();
    client.add_card(&owner, &card_n(&env, Genre::Pop, 999));
    let thousandth = env.cost_estimate().resources();

    assert_eq!(thousandth.memory_read_entries, first.memory_read_entries);
    assert_eq!(thousandth.disk_read_entries, first.disk_read_entries);
    assert_eq!(thousandth.write_entries, first.write_entries);
    // Only the grown id/position numbers differ in encoded size.
    assert!(thousandth.write_bytes <= first.write_bytes + 16);
    assert!(thousandth.disk_read_bytes <= first.disk_read_bytes + 16);
}

#[test]
fn join_round_rejects_when_full() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &2);
    let round_id = client.create_round(&owner, &Some(Genre::Pop), &1u64);
    for _ in 1..MAX_ROUND_PLAYERS {
        client.join_round(&Address::generate(&env), &round_id);
    }
    assert_eq!(client.get_players_round_count(&round_id), MAX_ROUND_PLAYERS);
    assert_eq!(
        client.try_join_round(&Address::generate(&env), &round_id),
        Err(Ok(Error::RoundFull.into()))
    );
}

#[test]
fn admin_and_gameplay_mutations_emit_events() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);

    client.set_cards_per_round(&owner, &2);
    assert_emitted(&env, &client.address, CardsPerRoundUpdated { value: 2 });

    let admin = Address::generate(&env);
    client.set_role(&owner, &admin, &Role::Admin, &true);
    assert_emitted(
        &env,
        &client.address,
        RoleUpdated {
            account: admin.clone(),
            role: Role::Admin,
            enabled: true,
        },
    );

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &1u64);
    client.start_round(&owner, &round_id);
    let card = client.next_card(&round_id);
    assert_emitted(
        &env,
        &client.address,
        CardDrawn {
            round_id,
            index: 0,
            card_id: card.card_id,
        },
    );

    let correct = client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone()));
    assert!(correct);
    assert_emitted(
        &env,
        &client.address,
        AnswerSubmitted {
            round_id,
            player: owner.clone(),
            correct: true,
        },
    );
}
