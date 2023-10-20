//! Processing for the compact format as specified here: https://github.com/ledgerwatch/erigon/blob/devel/docs/programmers_guide/witness_formal_spec.md

use std::{
    any::type_name,
    borrow::Borrow,
    collections::{linked_list::CursorMut, LinkedList},
    error::Error,
    fmt::{self, Display},
    io::{Cursor, Read},
    ops::Range,
};

use eth_trie_utils::partial_trie::HashedPartialTrie;
use ethereum_types::{H256, U256};
use serde::{de::DeserializeOwned, Deserialize};
use thiserror::Error;

use crate::{trace_protocol::TrieCompact, types::TrieRootHash};

pub type CompactParsingResult<T> = Result<T, CompactParsingError>;

type BranchMask = u32;

type Balance = U256;
type Nonce = U256;
type HasCode = bool;
type HasStorage = bool;

type HashValue = H256;
type RawValue = Vec<u8>;
type RawCode = Vec<u8>;

const MAX_WITNESS_ENTRIES_NEEDED_TO_MATCH_A_RULE: usize = 3;

#[derive(Debug, Error)]
pub enum CompactParsingError {
    #[error("Missing header")]
    MissingHeader,

    #[error("Invalid opcode operator (\"{0:x}\"")]
    InvalidOperator(u8),

    #[error("Reached the end of the byte stream when we still expected more data")]
    UnexpectedEndOfStream,

    #[error("Unable to parse an expected byte vector (error: {0})")]
    InvalidByteVector(String),

    #[error("Unable to parse the type \"{0}\" from cbor bytes {1}")]
    InvalidBytesForType(&'static str, String, String),

    #[error("Invalid block witness entries: {0:?}")]
    InvalidWitnessFormat(Vec<WitnessEntry>),

    #[error("There were multiple entries remaining after the compact block witness was processed (Remaining entries: {0:?})")]
    NonSingleEntryAfterProcessing(WitnessEntries),
}

#[derive(Clone, Debug, Deserialize)]
struct Key {
    is_even: bool,
    bytes: Vec<u8>,
}

impl<K: Borrow<[u8]>> From<K> for Key {
    fn from(_value: K) -> Self {
        todo!()
    }
}

#[derive(Debug, enumn::N)]
enum Opcode {
    Leaf = 0x00,
    Extension = 0x01,
    Branch = 0x02,
    Hash = 0x03,
    Code = 0x04,
    AccountLeaf = 0x05,
    EmptyRoot = 0x06,
}

#[derive(Clone, Debug)]
enum WitnessEntry {
    Instruction(Instruction),
    Node(NodeEntry),
}

// TODO: Ignore `NEW_TRIE` for now...
#[derive(Clone, Debug)]
enum Instruction {
    Leaf(Key, RawValue),
    Extension(Key),
    Branch(BranchMask),
    Hash(HashValue),
    Code(RawCode),
    AccountLeaf(Key, Nonce, Balance, HasCode, HasStorage),
    EmptyRoot,
}

impl From<Instruction> for WitnessEntry {
    fn from(v: Instruction) -> Self {
        Self::Instruction(v)
    }
}

#[derive(Clone, Debug)]
enum NodeEntry {
    Account(AccountNodeData),
    Code(Vec<u8>),
    Empty,
    Hash(HashValue),
    Leaf(Key, LeafNodeData),
    Extension(Key, Box<NodeEntry>),
    Value(ValueNodeData),
}

#[derive(Clone, Debug)]
struct ValueNodeData(Vec<u8>);

impl From<Vec<u8>> for ValueNodeData {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

#[derive(Clone, Debug)]
enum LeafNodeData {
    Value(ValueNodeData),
    Account(AccountNodeData),
}

#[derive(Clone, Debug)]
enum AccountNodeCode {
    CodeNode(Vec<u8>),
    HashNode(TrieRootHash),
}

#[derive(Clone, Debug)]
struct AccountNodeData {
    nonce: Nonce,
    balance: Balance,
    storage_root: Option<TrieRootHash>,
    account_node_code: Option<AccountNodeCode>,
}

impl AccountNodeData {
    fn new(
        nonce: Nonce,
        balance: Balance,
        storage_root: Option<TrieRootHash>,
        account_node_code: Option<AccountNodeCode>,
    ) -> Self {
        Self {
            nonce,
            balance,
            storage_root,
            account_node_code,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LeafData {
    key: Key,
    value: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct Header {
    version: u8,
}

impl Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Erigon block witness version {}", self.version)
    }
}

impl Header {
    pub(crate) fn version_is_compatible(&self, target_ver: u8) -> bool {
        self.version == target_ver
    }
}

#[derive(Debug)]
struct ParserState {
    entries: WitnessEntries,
}

impl ParserState {
    fn create_and_extract_header(
        witness_bytes_raw: Vec<u8>,
    ) -> CompactParsingResult<(Header, Self)> {
        let witness_bytes = WitnessBytes::new(witness_bytes_raw);
        let (header, entries) = witness_bytes.process_into_instructions_and_header()?;

        let p_state = Self { entries };

        Ok((header, p_state))
    }

    fn parse(self) -> CompactParsingResult<HashedPartialTrie> {
        let trie = self.parse_into_trie()?;
        Ok(trie)
    }

    fn parse_into_trie(mut self) -> CompactParsingResult<HashedPartialTrie> {
        let mut entry_buf = Vec::new();

        loop {
            let num_rules_applied = self.apply_rules_to_witness_entries(&mut entry_buf)?;

            if num_rules_applied == 0 {
                break;
            }
        }

        match self.entries.len() {
            1 => Self::create_partial_trie_from_remaining_witness_elem(self.entries.pop().unwrap()),
            _ => Err(CompactParsingError::NonSingleEntryAfterProcessing(
                self.entries,
            )),
        }
    }

    fn create_partial_trie_from_remaining_witness_elem(
        remaining_entry: WitnessEntry,
    ) -> CompactParsingResult<HashedPartialTrie> {
        todo!();
    }

    fn apply_rules_to_witness_entries(
        &mut self,
        entry_buf: &mut Vec<&WitnessEntry>,
    ) -> CompactParsingResult<usize> {
        let mut traverser = self.entries.create_collapsable_traverser();

        let mut tot_rules_applied = 0;

        while !traverser.at_end() {
            let num_rules_applied = Self::try_apply_rules_to_curr_entry(&mut traverser, entry_buf)?;
            tot_rules_applied += num_rules_applied;

            if num_rules_applied == 0 {
                // Unable to apply rule at current position, so advance the traverser.
                traverser.advance();
            }
        }

        Ok(tot_rules_applied)
    }

    fn try_apply_rules_to_curr_entry(
        traverser: &mut CollapsableWitnessEntryTraverser,
        buf: &mut Vec<&WitnessEntry>,
    ) -> CompactParsingResult<usize> {
        traverser.get_next_n_elems_into_buf(MAX_WITNESS_ENTRIES_NEEDED_TO_MATCH_A_RULE, buf);

        // TODO: There is a decent amount of code duplication with the matches and the
        // calls to `invalid_witness_err`. We should condense this...
        match buf[0] {
            WitnessEntry::Instruction(Instruction::Hash(h)) => {
                Self::traverser_replace_prev_n_nodes_entry_helper(1, traverser, NodeEntry::Hash(*h))
            }
            WitnessEntry::Instruction(Instruction::Leaf(k, v)) => {
                Self::traverser_replace_prev_n_nodes_entry_helper(
                    1,
                    traverser,
                    NodeEntry::Leaf(k.clone(), LeafNodeData::Value(v.clone().into())),
                )
            }
            WitnessEntry::Instruction(Instruction::Extension(k)) => {
                traverser.get_prev_n_elems_into_buf(1, buf);

                match buf[0] {
                    WitnessEntry::Node(node) => Self::traverser_replace_prev_n_nodes_entry_helper(
                        2,
                        traverser,
                        NodeEntry::Extension(k.clone(), Box::new(node.clone())),
                    ),
                    _ => Self::invalid_witness_err(2, TraverserDirection::Backwards, traverser),
                }
            }
            WitnessEntry::Instruction(Instruction::Code(c)) => {
                Self::traverser_replace_prev_n_nodes_entry_helper(
                    1,
                    traverser,
                    NodeEntry::Code(c.clone()),
                )
            }
            WitnessEntry::Instruction(Instruction::AccountLeaf(k, n, b, has_code, has_storage)) => {
                let (n_nodes_to_replace, account_node_code, s_root) = match (has_code, has_storage)
                {
                    (false, false) => Self::match_account_leaf_no_code_and_no_storage(),
                    (false, true) => {
                        Self::match_account_leaf_no_code_but_has_storage(traverser, buf)
                    }
                    (true, false) => {
                        Self::match_account_leaf_has_code_but_no_storage(traverser, buf)
                    }
                    (true, true) => Self::match_account_leaf_has_code_and_storage(traverser, buf),
                }?;

                let account_leaf_data = AccountNodeData::new(*n, *b, s_root, account_node_code);
                let leaf_node = WitnessEntry::Node(NodeEntry::Leaf(
                    k.clone(),
                    LeafNodeData::Account(account_leaf_data),
                ));
                traverser.replace_prev_n_entries_with_single_entry(n_nodes_to_replace, leaf_node);

                Ok(1)
            }
            WitnessEntry::Instruction(Instruction::Branch(_mask)) => {
                todo!()
            }
            _ => Self::invalid_witness_err(
                MAX_WITNESS_ENTRIES_NEEDED_TO_MATCH_A_RULE,
                TraverserDirection::Both,
                traverser,
            ),
        }
    }

    fn match_account_leaf_no_code_and_no_storage(
    ) -> CompactParsingResult<(usize, Option<AccountNodeCode>, Option<TrieRootHash>)> {
        Ok((0, None, None))
    }

    fn match_account_leaf_no_code_but_has_storage(
        traverser: &mut CollapsableWitnessEntryTraverser,
        buf: &mut Vec<&WitnessEntry>,
    ) -> CompactParsingResult<(usize, Option<AccountNodeCode>, Option<TrieRootHash>)> {
        traverser.get_prev_n_elems_into_buf(1, buf);

        match buf[0] {
            WitnessEntry::Node(node) => match Self::try_get_storage_hash_from_node(node) {
                Some(s_hash) => Ok((1, None, Some(s_hash))),
                None => Self::invalid_witness_err(1, TraverserDirection::Backwards, traverser),
            },
            _ => Self::invalid_witness_err(2, TraverserDirection::Backwards, traverser),
        }
    }

    fn match_account_leaf_has_code_but_no_storage(
        traverser: &mut CollapsableWitnessEntryTraverser,
        buf: &mut Vec<&WitnessEntry>,
    ) -> CompactParsingResult<(usize, Option<AccountNodeCode>, Option<TrieRootHash>)> {
        traverser.get_prev_n_elems_into_buf(1, buf);

        match buf[0] {
            WitnessEntry::Node(NodeEntry::Code(code)) => {
                Ok((1, Some(AccountNodeCode::CodeNode(code.clone())), None))
            }
            WitnessEntry::Node(NodeEntry::Hash(h)) => {
                Ok((1, Some(AccountNodeCode::HashNode(*h)), None))
            }
            _ => Self::invalid_witness_err(2, TraverserDirection::Backwards, traverser),
        }
    }

    fn match_account_leaf_has_code_and_storage(
        traverser: &mut CollapsableWitnessEntryTraverser,
        buf: &mut Vec<&WitnessEntry>,
    ) -> CompactParsingResult<(usize, Option<AccountNodeCode>, Option<TrieRootHash>)> {
        traverser.get_prev_n_elems_into_buf(2, buf);

        match buf[0..=1] {
            [WitnessEntry::Node(NodeEntry::Code(_c)), WitnessEntry::Node(_node)] => {
                todo!()
            }
            [WitnessEntry::Node(NodeEntry::Hash(_h)), WitnessEntry::Node(_node)] => {
                todo!()
            }
            _ => Self::invalid_witness_err(3, TraverserDirection::Backwards, traverser),
        }
    }

    fn try_get_storage_hash_from_node(_node: &NodeEntry) -> Option<TrieRootHash> {
        todo!()
    }

    fn invalid_witness_err<T>(
        n: usize,
        t_dir: TraverserDirection,
        traverser: &mut CollapsableWitnessEntryTraverser,
    ) -> CompactParsingResult<T> {
        let adjacent_elems_buf = match t_dir {
            TraverserDirection::Forwards => traverser.get_next_n_elems(n).cloned().collect(),
            TraverserDirection::Backwards => traverser.get_prev_n_elems(n).cloned().collect(),
            TraverserDirection::Both => todo!(),
        };

        Err(CompactParsingError::InvalidWitnessFormat(
            adjacent_elems_buf,
        ))
    }

    fn traverser_replace_prev_n_nodes_entry_helper(
        n: usize,
        traverser: &mut CollapsableWitnessEntryTraverser,
        entry: NodeEntry,
    ) -> CompactParsingResult<usize> {
        traverser.replace_prev_n_entries_with_single_entry(n, WitnessEntry::Node(entry));
        Ok(1)
    }
}

struct WitnessBytes {
    byte_cursor: CompactCursor,
    instrs: WitnessEntries,
}

impl WitnessBytes {
    fn new(witness_bytes: Vec<u8>) -> Self {
        Self {
            byte_cursor: CompactCursor::new(witness_bytes),
            instrs: WitnessEntries::default(),
        }
    }

    fn process_into_instructions_and_header(
        mut self,
    ) -> CompactParsingResult<(Header, WitnessEntries)> {
        let header = self.parse_header()?;

        // TODO
        loop {
            let instr = self.process_operator()?;
            self.instrs.push(instr.into());

            if self.byte_cursor.at_eof() {
                break;
            }
        }

        Ok((header, self.instrs))
    }

    fn process_operator(&mut self) -> CompactParsingResult<Instruction> {
        let opcode_byte = self.byte_cursor.read_byte()?;

        let opcode =
            Opcode::n(opcode_byte).ok_or(CompactParsingError::InvalidOperator(opcode_byte))?;

        self.process_data_following_opcode(opcode)?;

        todo!()
    }

    fn process_data_following_opcode(&mut self, opcode: Opcode) -> CompactParsingResult<()> {
        match opcode {
            Opcode::Leaf => self.process_leaf(),
            Opcode::Extension => self.process_extension(),
            Opcode::Branch => self.process_branch(),
            Opcode::Hash => self.process_hash(),
            Opcode::Code => self.process_code(),
            Opcode::AccountLeaf => self.process_leaf(),
            Opcode::EmptyRoot => self.process_empty_root(),
        }
    }

    fn process_leaf(&mut self) -> CompactParsingResult<()> {
        let key = self.byte_cursor.read_cbor_byte_array()?.into();
        let value_raw = self.byte_cursor.read_cbor_byte_array_to_vec()?;

        self.push_entry(Instruction::Leaf(key, value_raw));
        Ok(())
    }

    fn process_extension(&mut self) -> CompactParsingResult<()> {
        let key = self.byte_cursor.read_cbor_byte_array()?.into();

        self.push_entry(Instruction::Extension(key));
        Ok(())
    }

    fn process_branch(&mut self) -> CompactParsingResult<()> {
        let mask = self.byte_cursor.read_t()?;

        self.push_entry(Instruction::Branch(mask));
        Ok(())
    }

    fn process_hash(&mut self) -> CompactParsingResult<()> {
        let hash = self.byte_cursor.read_t()?;

        self.push_entry(Instruction::Hash(hash));
        Ok(())
    }

    fn process_code(&mut self) -> CompactParsingResult<()> {
        let code = self.byte_cursor.read_t()?;

        self.push_entry(Instruction::Code(code));
        Ok(())
    }

    fn process_account_leaf(&mut self) -> CompactParsingResult<()> {
        let key = self.byte_cursor.read_cbor_byte_array()?.into();
        let nonce = self.byte_cursor.read_t()?;
        let balance = self.byte_cursor.read_t()?;
        let has_code = self.byte_cursor.read_t()?;
        let has_storage = self.byte_cursor.read_t()?;

        self.push_entry(Instruction::AccountLeaf(
            key,
            nonce,
            balance,
            has_code,
            has_storage,
        ));

        Ok(())
    }

    fn process_empty_root(&mut self) -> CompactParsingResult<()> {
        self.push_entry(Instruction::EmptyRoot);
        Ok(())
    }

    fn push_entry(&mut self, instr: Instruction) {
        self.instrs.push(instr.into())
    }

    fn parse_header(&mut self) -> CompactParsingResult<Header> {
        let h_byte = self
            .byte_cursor
            .read_byte()
            .map_err(|_| CompactParsingError::MissingHeader)?;

        Ok(Header { version: h_byte })
    }
}

#[derive(Debug)]
struct CompactCursor {
    intern: Cursor<Vec<u8>>,
    temp_buf: Vec<u8>,
}

impl CompactCursor {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            intern: Cursor::new(bytes),
            temp_buf: Vec::default(),
        }
    }

    fn read_t<T: DeserializeOwned>(&mut self) -> CompactParsingResult<T> {
        let starting_pos = self.intern.position();

        ciborium::from_reader(&mut self.intern).map_err(move |err| {
            let ending_pos = self.intern.position();
            let type_bytes = self.intern.clone().into_inner()
                [starting_pos as usize..ending_pos as usize]
                .to_vec();
            let type_bytes_hex = hex::encode(type_bytes);

            CompactParsingError::InvalidBytesForType(
                type_name::<T>(),
                type_bytes_hex,
                err.to_string(),
            )
        })
    }

    fn read_byte(&mut self) -> CompactParsingResult<u8> {
        let mut single_byte_buf = [0];

        // Assume this is always caused by hitting the end of the stream?
        self.intern
            .read_exact(&mut single_byte_buf)
            .map_err(|_err| CompactParsingError::UnexpectedEndOfStream)?;

        Ok(single_byte_buf[0])
    }

    fn read_cbor_byte_array(&mut self) -> CompactParsingResult<&[u8]> {
        self.temp_buf.clear();
        Self::ciborium_byte_vec_err_reader_res_to_parsing_res(ciborium_io::Read::read_exact(
            &mut self.intern,
            &mut self.temp_buf,
        ))?;

        Ok(&self.temp_buf)
    }

    fn read_cbor_byte_array_to_vec(&mut self) -> CompactParsingResult<Vec<u8>> {
        Self::ciborium_byte_vec_err_reader_res_to_parsing_res(ciborium::from_reader(
            &mut self.intern,
        ))
    }

    fn ciborium_byte_vec_err_reader_res_to_parsing_res<T, E: Error>(
        res: Result<T, E>,
    ) -> CompactParsingResult<T> {
        res.map_err(|err| CompactParsingError::InvalidByteVector(err.to_string()))
    }

    fn at_eof(&self) -> bool {
        self.intern.position() as usize == self.intern.get_ref().len()
    }
}

/// We kind of want a wrapper around the actual data structure I think since
/// there's a good chance this will change a few times in the future.
#[derive(Debug, Default)]
struct WitnessEntries {
    // Yeah a LL is actually (unfortunately) a very good choice here. We will be doing a ton of
    // inserts mid-list, and the list can get very large. There might be a better choice for a data
    // structure, but for now, this will make performance not scale exponentially with list
    // size.
    intern: LinkedList<WitnessEntry>,
}

impl WitnessEntries {
    fn push(&mut self, _entry: WitnessEntry) {
        todo!()
    }

    fn pop(&mut self) -> Option<WitnessEntry> {
        todo!()
    }

    fn replace_entries_with_single_entry(
        &mut self,
        _idxs_to_replace: Range<usize>,
        _entry_to_replace_with: WitnessEntry,
    ) {
        todo!()
    }

    fn create_collapsable_traverser(&mut self) -> CollapsableWitnessEntryTraverser {
        todo!()
    }

    fn len(&self) -> usize {
        self.intern.len()
    }
}

// It's not quite an iterator, so this is the next best name that I can come up
// with.
struct CollapsableWitnessEntryTraverser<'a> {
    entries: &'a mut WitnessEntries,
    entry_cursor: CursorMut<'a, WitnessEntry>,
}

impl<'a> CollapsableWitnessEntryTraverser<'a> {
    fn advance(&mut self) {
        todo!()
    }

    fn get_next_n_elems(&self, _n: usize) -> impl Iterator<Item = &WitnessEntry> {
        // TODO
        std::iter::empty()
    }

    fn get_prev_n_elems(&self, _n: usize) -> impl Iterator<Item = &WitnessEntry> {
        // TODO
        std::iter::empty()
    }

    /// Get the previous `n` elements into a buf. Note that this does not
    /// include the element that we are currently pointing to.
    fn get_prev_n_elems_into_buf(&self, _n: usize, _buf: &mut Vec<&WitnessEntry>) {
        todo!()
    }

    /// Get the next `n` elements into a buf. Note that this includes the
    /// element that we are currently pointing to.
    fn get_next_n_elems_into_buf(&self, _n: usize, _buf: &mut Vec<&WitnessEntry>) {
        todo!()
    }

    fn replace_next_n_entries_with_single_entry(&mut self, n: usize, entry: WitnessEntry) {
        for _ in 0..n {
            self.entry_cursor.remove_current();
        }

        self.entry_cursor.insert_after(entry)
    }

    fn replace_prev_n_entries_with_single_entry(&mut self, _n: usize, _entry: WitnessEntry) {
        todo!()
    }

    fn at_end(&self) -> bool {
        self.entry_cursor.as_cursor().peek_next().is_none()
    }
}

#[derive(Debug)]
enum TraverserDirection {
    Forwards,
    Backwards,
    Both,
}

pub(crate) fn process_compact_prestate(
    state: TrieCompact,
) -> CompactParsingResult<(Header, HashedPartialTrie)> {
    let (header, parser) = ParserState::create_and_extract_header(state.bytes)?;
    let trie = parser.parse()?;

    Ok((header, trie))
}
