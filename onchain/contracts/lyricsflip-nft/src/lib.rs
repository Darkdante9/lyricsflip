#![no_std]

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Env, String,
};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NftMinted {
    #[topic]
    pub token_id: u128,
    pub recipient: Address,
}

/// SEP-0050 `transfer` event, emitted by `transfer` and `transfer_from`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub token_id: u128,
}

/// SEP-0050 `approve` event.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approve {
    #[topic]
    pub approver: Address,
    #[topic]
    pub token_id: u128,
    pub approved: Address,
    pub live_until_ledger: u32,
}

/// SEP-0050 `approve_for_all` event. A `live_until_ledger` of 0 revokes.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveForAll {
    #[topic]
    pub owner: Address,
    pub operator: Address,
    pub live_until_ledger: u32,
}

/// An approval that is valid up to and including `live_until_ledger`.
#[contracttype]
#[derive(Clone)]
struct Approval {
    approved: Address,
    live_until_ledger: u32,
}

/// Ported from `onchain/src/contracts/lyricsflipNFT.cairo`. Implements the
/// SEP-0050 non-fungible interface (balance, transfer, approvals) by hand,
/// mirroring OpenZeppelin `stellar-non-fungible`'s method names and events,
/// on top of minter-gated `mint` and a monotonically increasing token id.
///
/// Clients map on the numeric values, so never renumber or reuse a code; keep
/// `onchain/README.md` and `test::error_codes_are_stable` in sync.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Defensive only: `__constructor` runs exactly once per deployment.
    AlreadyInitialized = 1,
    NotMinter = 2,
    /// Defensive only: token ids come from a monotonic counter, so a
    /// collision should never happen.
    TokenAlreadyExists = 3,
    TokenDoesNotExist = 4,
    /// `from` does not own the token.
    IncorrectOwner = 5,
    /// The spender/approver is neither the owner nor an approved operator.
    InsufficientApproval = 6,
    /// `live_until_ledger` is in the past (and not 0 for a revoke).
    InvalidLiveUntilLedger = 7,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Owner,
    Minter,
    TokenName,
    TokenSymbol,
    BaseUri,
    TokenCount,
    TokenOwner(u128),
    Balance(Address),
    Approval(u128),
    ApprovalForAll((Address, Address)),
}

#[contract]
pub struct LyricsFlipNFT;

#[contractimpl]
impl LyricsFlipNFT {
    pub fn __constructor(
        env: Env,
        owner: Address,
        minter: Address,
        token_name: String,
        token_symbol: String,
        base_uri: String,
    ) {
        if env.storage().instance().has(&DataKey::Owner) {
            panic_with_error!(env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Minter, &minter);
        env.storage()
            .instance()
            .set(&DataKey::TokenName, &token_name);
        env.storage()
            .instance()
            .set(&DataKey::TokenSymbol, &token_symbol);
        env.storage().instance().set(&DataKey::BaseUri, &base_uri);
        env.storage().instance().set(&DataKey::TokenCount, &0u128);
    }

    pub fn mint(env: Env, caller: Address, recipient: Address) -> u128 {
        caller.require_auth();

        let minter: Address = env.storage().instance().get(&DataKey::Minter).unwrap();
        if caller != minter {
            panic_with_error!(env, Error::NotMinter);
        }

        let count: u128 = env
            .storage()
            .instance()
            .get(&DataKey::TokenCount)
            .unwrap_or(0);
        let token_id = count + 1;

        if env
            .storage()
            .persistent()
            .has(&DataKey::TokenOwner(token_id))
        {
            panic_with_error!(env, Error::TokenAlreadyExists);
        }

        env.storage()
            .persistent()
            .set(&DataKey::TokenOwner(token_id), &recipient);
        Self::add_balance(&env, &recipient, 1);
        env.storage()
            .instance()
            .set(&DataKey::TokenCount, &token_id);

        NftMinted {
            token_id,
            recipient,
        }
        .publish(&env);

        token_id
    }

    pub fn owner_of(env: Env, token_id: u128) -> Address {
        env.storage()
            .persistent()
            .get(&DataKey::TokenOwner(token_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::TokenDoesNotExist))
    }

    pub fn balance(env: Env, owner: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(owner))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, token_id: u128) {
        from.require_auth();
        Self::do_transfer(&env, &from, &to, token_id);
    }

    /// Transfers on behalf of `from` by a `spender` approved for the token
    /// or for all of `from`'s tokens.
    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, token_id: u128) {
        spender.require_auth();
        let is_token_approved = Self::get_approved(env.clone(), token_id) == Some(spender.clone());
        if spender != from
            && !is_token_approved
            && !Self::is_approved_for_all(env.clone(), from.clone(), spender)
        {
            panic_with_error!(env, Error::InsufficientApproval);
        }
        Self::do_transfer(&env, &from, &to, token_id);
    }

    /// Approves `approved` to transfer `token_id` until `live_until_ledger`.
    /// `approver` must be the owner or an operator approved for all.
    pub fn approve(
        env: Env,
        approver: Address,
        approved: Address,
        token_id: u128,
        live_until_ledger: u32,
    ) {
        approver.require_auth();
        let owner = Self::owner_of(env.clone(), token_id);
        if approver != owner && !Self::is_approved_for_all(env.clone(), owner, approver.clone()) {
            panic_with_error!(env, Error::InsufficientApproval);
        }
        let key = DataKey::Approval(token_id);
        if live_until_ledger == 0 {
            env.storage().persistent().remove(&key);
        } else {
            Self::assert_live_until(&env, live_until_ledger);
            env.storage().persistent().set(
                &key,
                &Approval {
                    approved: approved.clone(),
                    live_until_ledger,
                },
            );
        }
        Approve {
            approver,
            token_id,
            approved,
            live_until_ledger,
        }
        .publish(&env);
    }

    /// Approves `operator` for all of `owner`'s tokens until
    /// `live_until_ledger`; 0 revokes.
    pub fn approve_for_all(env: Env, owner: Address, operator: Address, live_until_ledger: u32) {
        owner.require_auth();
        let key = DataKey::ApprovalForAll((owner.clone(), operator.clone()));
        if live_until_ledger == 0 {
            env.storage().persistent().remove(&key);
        } else {
            Self::assert_live_until(&env, live_until_ledger);
            env.storage().persistent().set(&key, &live_until_ledger);
        }
        ApproveForAll {
            owner,
            operator,
            live_until_ledger,
        }
        .publish(&env);
    }

    pub fn get_approved(env: Env, token_id: u128) -> Option<Address> {
        env.storage()
            .persistent()
            .get::<_, Approval>(&DataKey::Approval(token_id))
            .filter(|a| a.live_until_ledger >= env.ledger().sequence())
            .map(|a| a.approved)
    }

    pub fn is_approved_for_all(env: Env, owner: Address, operator: Address) -> bool {
        env.storage()
            .persistent()
            .get::<_, u32>(&DataKey::ApprovalForAll((owner, operator)))
            .is_some_and(|live_until| live_until >= env.ledger().sequence())
    }

    pub fn token_name(env: Env) -> String {
        env.storage().instance().get(&DataKey::TokenName).unwrap()
    }

    pub fn token_symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::TokenSymbol).unwrap()
    }

    pub fn base_uri(env: Env) -> String {
        env.storage().instance().get(&DataKey::BaseUri).unwrap()
    }

    pub fn token_count(env: Env) -> u128 {
        env.storage()
            .instance()
            .get(&DataKey::TokenCount)
            .unwrap_or(0)
    }

    fn do_transfer(env: &Env, from: &Address, to: &Address, token_id: u128) {
        if Self::owner_of(env.clone(), token_id) != *from {
            panic_with_error!(env, Error::IncorrectOwner);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Approval(token_id));
        env.storage()
            .persistent()
            .set(&DataKey::TokenOwner(token_id), to);
        Self::add_balance(env, from, -1);
        Self::add_balance(env, to, 1);
        Transfer {
            from: from.clone(),
            to: to.clone(),
            token_id,
        }
        .publish(env);
    }

    fn add_balance(env: &Env, owner: &Address, delta: i32) {
        let balance = Self::balance(env.clone(), owner.clone());
        env.storage().persistent().set(
            &DataKey::Balance(owner.clone()),
            &balance.checked_add_signed(delta).unwrap(),
        );
    }

    fn assert_live_until(env: &Env, live_until_ledger: u32) {
        if live_until_ledger < env.ledger().sequence() {
            panic_with_error!(env, Error::InvalidLiveUntilLedger);
        }
    }
}
