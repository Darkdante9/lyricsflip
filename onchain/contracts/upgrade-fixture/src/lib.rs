//! Stand-in "v2" code for the upgrade tests. Its built WASM is committed as
//! `onchain/fixtures/upgrade_v2.wasm` so `cargo test` needs no WASM build;
//! rebuild it with `cargo build --target wasm32v1-none --release` and copy
//! `target/wasm32v1-none/release/upgrade_fixture.wasm` over it.
#![no_std]

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct UpgradeFixture;

#[contractimpl]
impl UpgradeFixture {
    pub fn version() -> u32 {
        2
    }
}
