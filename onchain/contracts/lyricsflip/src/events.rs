use crate::types::{Genre, Role};
use soroban_sdk::{contractevent, Address, Map, Vec};

use crate::types::{Milestone, Role};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundCreated {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub admin: Address,
    pub created_time: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundStarted {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub admin: Address,
    pub start_time: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundJoined {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub player: Address,
    pub joined_time: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerReady {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub player: Address,
    pub ready_time: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundCompleted {
    #[topic]
    pub round_id: u64,
    pub winners: Vec<Address>,
    pub scores: Map<Address, u64>,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardAdded {
    #[topic]
    pub card_id: u64,
    #[topic]
    pub genre: Genre,
pub struct RoundLeft {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub player: Address,
    pub refunded: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardUpdated {
    #[topic]
    pub card_id: u64,
    #[topic]
    pub genre: Genre,
pub struct RoundCancelled {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub cancelled_by: Address,
    /// Players whose wager was refunded (everyone still in the round).
    pub refunded_players: Vec<Address>,
    pub refund_per_player: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardRemoved {
    #[topic]
    pub card_id: u64,
pub struct RewardClaimed {
    #[topic]
    pub player: Address,
    pub milestone: Milestone,
    pub token_id: u128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardDrawn {
    #[topic]
    pub round_id: u64,
    pub index: u32,
    pub card_id: u64,
pub struct RoleUpdated {
    #[topic]
    pub address: Address,
    pub role: Role,
    pub enabled: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnswerSubmitted {
    #[topic]
    pub round_id: u64,
    #[topic]
    pub player: Address,
    pub correct: bool,
    pub points: u64,
pub struct OwnershipTransferStarted {
    #[topic]
    pub owner: Address,
    #[topic]
    pub pending_owner: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleUpdated {
    #[topic]
    pub account: Address,
    pub role: Role,
    pub enabled: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardsPerRoundUpdated {
    pub value: u32,
pub struct OwnershipTransferred {
    #[topic]
    pub old_owner: Address,
    #[topic]
    pub new_owner: Address,
}
