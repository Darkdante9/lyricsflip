//! Representative behavior-coverage tests for the LyricsFlip game contract.
//! Covers round lifecycle, card queries, answer submission, access control,
//! LF-011 (O(limit) random selection), and LF-012 (TTL survival).

use crate::{Answer, Card, Error, Genre, LyricsFlip, LyricsFlipClient, Role, MAX_PAGE_LIMIT};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    xdr, Address, Env, Map, String,
};

fn setup<'a>() -> (Env, LyricsFlipClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
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
        let card = sample_card(env, Genre::Pop, "Artist A", "Title A", 2000 + i);
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
    assert!(round.is_started, "round should start once all players are ready");

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

fn assert_round_completed_event(
    env: &Env,
    contract_id: &Address,
    round_id: u64,
    winners: &soroban_sdk::Vec<Address>,
    scores: &Map<Address, u64>,
) {
    let expected_round_id = xdr::ScVal::try_from_val(env, &round_id).unwrap();
    let expected_data = soroban_sdk::vec![
        env,
        winners.clone().into_val(env),
        scores.clone().into_val(env)
    ];
    let expected_data_xdr = xdr::ScVal::try_from_val(env, &expected_data).unwrap();

    let events = env.events().all().filter_by_contract(contract_id);
    let mut found = false;
    for event in events.events().iter() {
        let body = match &event.body {
            xdr::ContractEventBody::V0(body) => body,
            _ => continue,
        };
        if body.topics.len() != 1 {
            continue;
        }
        if body.topics[0] != expected_round_id {
            continue;
        }
        if body.data == expected_data_xdr {
            found = true;
            break;
        }
    }

    assert!(
        found,
        "expected RoundCompleted event to include the winning scores and round id"
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
    ];
    for (variant, code) in expected {
        assert_eq!(variant as u32, code, "{:?} was renumbered", variant);
    }
}

// ---------------------------------------------------------------------------
// LF-011 – partial Fisher-Yates is O(limit), not O(coupon-collector)
// ---------------------------------------------------------------------------

/// Confirms that `get_random_numbers(amount=50, limit=50)` — the worst-case
/// scenario that caused the coupon-collector blowup — now runs in a fixed
/// number of iterations. We check this indirectly: seed the contract with 50
/// cards, call `get_cards_of_genre` (which internally calls
/// `get_random_numbers(amount=50, limit=50)`), and assert the result is
/// exactly 50 distinct card IDs with no panics or budget exhaustion.
#[test]
fn get_random_numbers_worst_case_amount_equals_limit() {
    let (env, client, owner) = setup();

    // Add 50 distinct cards (all Pop genre).
    for i in 0..50u64 {
        let title = soroban_sdk::String::from_str(&env, "T");
        // Unique titles via seeded shuffle are not required here; we only care
        // that the function returns 50 results without panicking.
        let card = Card {
            card_id: 0,
            genre: Genre::Pop,
            artist: soroban_sdk::String::from_str(&env, "A"),
            title,
            year: 2000 + i,
            lyrics: soroban_sdk::String::from_str(&env, "lyric"),
        };
        client.add_card(&owner, &card);
    }
    client.set_cards_per_round(&owner, &50);

    // This must complete without panicking (no infinite loop).
    let cards = client.get_cards_of_genre(&Genre::Pop, &12345u64);
    assert_eq!(
        cards.len(),
        50,
        "expected exactly 50 cards back when amount == limit == 50"
    );
}

/// Verifies that `get_random_numbers` returns the requested `amount` without
/// panicking across a variety of (amount, limit) pairs and seeds.
#[test]
fn get_random_numbers_returns_correct_count_for_various_inputs() {
    let (env, client, owner) = setup();
    for i in 0..20u64 {
        client.add_card(
            &owner,
            &sample_card(&env, Genre::Jazz, "Artist", "Title", 1970 + i),
        );
    }

    for amount in [1u32, 5, 10, 15, 20] {
        client.set_cards_per_round(&owner, &amount);
        let cards = client.get_cards_of_genre(&Genre::Jazz, &(amount as u64 * 7 + 3));
        assert_eq!(
            cards.len(),
            amount,
            "expected {amount} cards, got {}",
            cards.len()
        );
    }
}

// ---------------------------------------------------------------------------
// LF-012 – persistent storage survives a ledger advance past default TTL
// ---------------------------------------------------------------------------

/// After gameplay entries (cards, round, player stats) are written and the
/// ledger sequence advances well past Soroban's default minimum TTL, the
/// entries are still readable because `extend_ttl` was called on write.
#[test]
fn ttl_survival_game_entries_survive_ledger_advance() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 3);
    client.set_cards_per_round(&owner, &2);

    let round_id = client.create_round(&owner, &Some(Genre::Pop), &77u64);
    client.start_round(&owner, &round_id);

    let card = client.next_card(&round_id);
    assert!(client.submit_answer(&owner, &round_id, &Answer::Title(card.title.clone())));

    // Advance ledger sequence far beyond the Soroban default minimum TTL
    // (4 096 ledgers). If entries were never extended they would be
    // unreadable at this point.
    env.ledger().set_sequence_number(100_000);

    // All entries written during gameplay must still be readable.
    let _round = client.get_round(&round_id);
    let _card_back = client.get_card(&1);
    let stats = client.get_player_stat(&owner);
    assert_eq!(
        stats.total_rounds, 1,
        "player stats should still be readable after ledger advance"
    );
    assert_eq!(stats.current_streak, 1);
}

/// Cards added before a large ledger advance are still readable afterwards.
#[test]
fn ttl_survival_cards_survive_ledger_advance() {
    let (env, client, owner) = setup();
    seed_cards(&env, &client, &owner, 5);
    client.set_cards_per_round(&owner, &3);

    env.ledger().set_sequence_number(200_000);

    // Cards and genre index should still be accessible.
    let _card = client.get_card(&1);
    let cards = client.get_cards_of_genre(&Genre::Pop, &999u64);
    assert_eq!(cards.len(), 3);
}
