//! Examples of constructing [`PartialTrie`]s for actual Ethereum tries.
//!
//! It's a bit difficult to ensure that nodes in Ethereum tries are RLP encoded
//! correctly. This library encodes keys being passed into the trie, but does
//! not apply any encoding to the values themselves.
//!
//! Also note that due to RLP encoding, the underlying integer types (with the
//! exception of hash types like `H256`) don't affect the hash generated due to
//! the RLP encoding truncating any leading zeros.

use std::ops::RangeInclusive;

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::rpc::types::serde_helpers::quantity::vec;
use alloy_rlp::{Encodable, RlpEncodable};
use mpt_trie::partial_trie::PartialTrie;
use mpt_trie::trie_ops::TrieOpResult;
use mpt_trie::utils::TryFromIterator;
use mpt_trie::{
    nibbles::Nibbles,
    partial_trie::{HashedPartialTrie, StandardTrie},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
//use rlp::Encodable;

const RANGE_OF_STORAGE_ENTRIES_AN_ACCOUNT_CAN_HAVE: RangeInclusive<usize> = 0..=10;
const NUM_ACCOUNTS_TO_GEN: usize = 100;

type HashedAccountAddr = B256;
type AccountAddr = Address;

/// Eth test account entry. As a separate struct to allow easy RLP encoding.
#[derive(Debug, RlpEncodable)]
struct StateTrieEntry {
    nonce: U256,
    balance: U256,
    storage_root: B256,
    code_hash: B256,
}

fn main() -> TrieOpResult<()> {
    let mut rng = StdRng::seed_from_u64(0);

    let (account_entries, account_storage_tries): (Vec<_>, Vec<_>) = (0..NUM_ACCOUNTS_TO_GEN)
        .map(|_| generate_fake_account_and_storage_trie(&mut rng))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .unzip();

    let _state_trie = StandardTrie::try_from_iter(account_entries.into_iter().map(|(k, acc)| {
        let mut rlp_bytes = vec![];
        acc.encode(&mut rlp_bytes);
        (Nibbles::from_h256_be(k), rlp_bytes)
    }))?;

    let _account_storage_tries: Vec<(AccountAddr, HashedPartialTrie)> = account_storage_tries;

    Ok(())

    // TODO: Generate remaining tries...
}

fn generate_fake_account_and_storage_trie(
    rng: &mut StdRng,
) -> TrieOpResult<(
    (HashedAccountAddr, StateTrieEntry),
    (AccountAddr, HashedPartialTrie),
)> {
    let account_addr: Address = Address::random();
    let hashed_account_addr = keccak256(account_addr);

    let account_storage_trie = generate_fake_account_storage_trie(rng)?;

    let acc_entry = StateTrieEntry {
        nonce: gen_u256(rng),
        balance: gen_u256(rng),
        storage_root: account_storage_trie.hash(),
        code_hash: B256::random(), /* For the test, the contract code does not exist, so we can
                                    * just "fake" it here. */
    };

    Ok((
        (hashed_account_addr, acc_entry),
        (account_addr, account_storage_trie),
    ))
}

fn generate_fake_account_storage_trie(rng: &mut StdRng) -> TrieOpResult<HashedPartialTrie> {
    let num_storage_entries = rng.gen_range(RANGE_OF_STORAGE_ENTRIES_AN_ACCOUNT_CAN_HAVE);

    HashedPartialTrie::try_from_iter((0..num_storage_entries).map(|_| {
        let hashed_storage_addr = Nibbles::from_h256_le(B256::random());
        let mut storage_data = vec![0u8; 32];
        gen_u256(rng).encode(&mut storage_data);

        (hashed_storage_addr, storage_data)
    }))
}

fn gen_u256(rng: &mut StdRng) -> U256 {
    U256::from_limbs(rng.gen::<[u64; 4]>())
}
