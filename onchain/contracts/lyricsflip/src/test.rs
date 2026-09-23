//! Representative behavior-coverage tests ported from
//! `onchain/src/tests/test_lyricsflip.cairo` (round lifecycle, card queries,
//! answer submission, and access control). These are written against the
//! soroban-sdk 22 testutils API but have not been `cargo test`-verified in
//! this environment (no Rust toolchain available) — run `cargo test` inside
//! `onchain/` before relying on them.

use crate::{Answer, Card, Genre, LyricsFlip, LyricsFlipClient, Role};
use soroban_sdk::{
    testutils::{Address as _, Events},
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
}
