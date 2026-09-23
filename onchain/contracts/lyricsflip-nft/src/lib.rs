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

/// Ported from `onchain/src/contracts/lyricsflipNFT.cairo`. Soroban has no
/// ERC721-equivalent standard bundled the way OpenZeppelin Cairo provides
/// one, and no `SRC5`-style interface introspection, so this keeps only the
/// surface the original contract actually used: minter-gated `mint`, a
/// monotonically increasing token id, and per-token ownership.
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
}

// ---------------------------------------------------------------------------
// LF-012 – TTL policy (mirrors lyricsflip game contract)
// ---------------------------------------------------------------------------
pub const DAY_IN_LEDGERS: u32 = 17_280;
pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
pub const LIFETIME_THRESHOLD: u32 = 7 * DAY_IN_LEDGERS;

#[inline]
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

#[inline]
fn bump_persistent<K: soroban_sdk::TryIntoVal<Env, soroban_sdk::Val>>(env: &Env, key: &K) {
    env.storage()
        .persistent()
        .extend_ttl(key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
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
        bump_instance(&env);
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
        bump_persistent(&env, &DataKey::TokenOwner(token_id));

        env.storage()
            .instance()
            .set(&DataKey::TokenCount, &token_id);
        bump_instance(&env);

        NftMinted {
            token_id,
            recipient,
        }
        .publish(&env);

        token_id
    }

    pub fn owner_of(env: Env, token_id: u128) -> Address {
        let owner: Address = env
            .storage()
            .persistent()
            .get(&DataKey::TokenOwner(token_id))
            .unwrap_or_else(|| panic_with_error!(env, Error::TokenDoesNotExist));
        bump_persistent(&env, &DataKey::TokenOwner(token_id));
        owner
    }

    pub fn token_name(env: Env) -> String {
        bump_instance(&env);
        env.storage().instance().get(&DataKey::TokenName).unwrap()
    }

    pub fn token_symbol(env: Env) -> String {
        bump_instance(&env);
        env.storage().instance().get(&DataKey::TokenSymbol).unwrap()
    }

    pub fn base_uri(env: Env) -> String {
        bump_instance(&env);
        env.storage().instance().get(&DataKey::BaseUri).unwrap()
    }

    pub fn token_count(env: Env) -> u128 {
        bump_instance(&env);
        env.storage()
            .instance()
            .get(&DataKey::TokenCount)
            .unwrap_or(0)
    }
}
