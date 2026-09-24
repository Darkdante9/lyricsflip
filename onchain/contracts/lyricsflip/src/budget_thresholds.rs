//! Resource-budget ceilings for `test_budget`. Measured with 500 cards and
//! 8 players; raise a value only with a matching note in the PR.
//!
//! Soroban per-transaction limits are 100M CPU instructions and 40 MiB of
//! memory, so every ceiling stays well below them.

pub const CREATE_ROUND_MAX_CPU: u64 = 40_000_000;
pub const CREATE_ROUND_MAX_MEM: u64 = 10_000_000;

pub const START_ROUND_MAX_CPU: u64 = 20_000_000;
pub const START_ROUND_MAX_MEM: u64 = 5_000_000;

pub const BUILD_QUESTION_CARD_MAX_CPU: u64 = 40_000_000;
pub const BUILD_QUESTION_CARD_MAX_MEM: u64 = 10_000_000;
