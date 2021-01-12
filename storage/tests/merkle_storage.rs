// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::convert::TryInto;
use std::error::Error;
use std::collections::VecDeque;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use crypto::hash::{HashType, BlockHash};
use rocksdb::{DB, Cache, Options};

use storage::*;
use in_memory::KVStore;
use context_action_storage::ContextAction;
use merkle_storage::{MerkleStorage, MerkleError, Entry, check_entry_hash};
use persistent::{PersistentStorage, CommitLogSchema, DbConfiguration, KeyValueSchema, open_cl, open_kv_readonly};
use persistent::sequence::Sequences;

// fn get_cycles_for_block(persistent_storage: &PersistentStorage, context_hash: &ContextHash) -> i32 {
//     let tezedge_context = TezedgeContext::new(
//         // BlockStorage::new(&persistent_storage),
//         BlockStorage::new(persistent_storage),
//         persistent_storage.merkle(),
//     );
//     let protocol_hash = tezedge_context.get_key_from_history(&context_hash, &context_key!("protocol")).unwrap();
//     let constants_data = tezedge_context.get_key_from_history(&context_hash, &context_key!("data/v1/constants")).unwrap();
//     let constants = tezos_messages::protocol::get_constants_for_rpc(&constants_data, protocol_hash).unwrap().unwrap();

//     match constants.get("blocks_per_cycle") {
//         Some(UniversalValue::Number(value)) => *value,
//         _ => panic!(4096),
//     }
// }

fn init_persistent_storage() -> PersistentStorage {
    // Parses config + cli args
    // let env = crate::configuration::Environment::from_args();
    let db_path = "/tmp/tezedge/light-node";

    let schemas = vec![
        block_storage::BlockPrimaryIndex::name(),
        block_storage::BlockByLevelIndex::name(),
        block_storage::BlockByContextHashIndex::name(),
        BlockMetaStorage::name(),
        OperationsStorage::name(),
        OperationsMetaStorage::name(),
        context_action_storage::ContextActionByBlockHashIndex::name(),
        context_action_storage::ContextActionByContractIndex::name(),
        context_action_storage::ContextActionByTypeIndex::name(),
        ContextActionStorage::name(),
        SystemStorage::name(),
        Sequences::name(),
        MempoolStorage::name(),
        ChainMetaStorage::name(),
        PredecessorStorage::name(),
    ];

    let opts = DbConfiguration::default();
    let rocks_db = Arc::new(
        open_kv_readonly(db_path, schemas, &opts).unwrap()
    );
    let commit_logs = match open_cl(db_path, vec![BlockStorage::descriptor()]) {
        Ok(commit_logs) => Arc::new(commit_logs),
        Err(e) => panic!(e),
    };

    PersistentStorage::new(rocks_db, commit_logs)
}

struct BlocksIterator {
    block_storage: BlockStorage,
    blocks: std::vec::IntoIter<BlockHeaderWithHash>,
    last_hash: Option<BlockHash>,
    limit: usize,
}

impl BlocksIterator {
    pub fn new(block_storage: BlockStorage, start_block_hash: &BlockHash, limit: usize) -> Self {
        let blocks = Self::get_blocks_after_block(&block_storage, &start_block_hash, limit).into_iter();
        Self { block_storage, blocks, limit, last_hash: None }
    }

    fn get_blocks_after_block(
        block_storage: &BlockStorage,
        block_hash: &BlockHash,
        limit: usize
    ) -> Vec<BlockHeaderWithHash> {
        block_storage.get_multiple_without_json(block_hash, limit).unwrap_or(vec![])
    }
}

impl Iterator for BlocksIterator {
    type Item = BlockHeaderWithHash;

    fn next(&mut self) -> Option<Self::Item> {
        match self.blocks.next() {
            Some(block) => {
                self.last_hash = Some(block.hash.clone());
                Some(block)
            }
            None => {
                let last_hash = self.last_hash.take();
                if last_hash.is_none() {
                    return None;
                }
                let new_blocks = Self::get_blocks_after_block(
                    &self.block_storage,
                    &last_hash.unwrap(),
                    self.limit,
                );

                if new_blocks.len() == 0 {
                    return None;
                }
                self.blocks = new_blocks.into_iter();
                self.next()
            }
        }
    }
}

#[test]
fn test_merkle_storage_gc() {
    let genesis_block_hash = HashType::BlockHash.b58check_to_hash("BLockGenesisGenesisGenesisGenesisGenesis355e8bjkYPv").unwrap();
    let blocks_limit = 256;
    let persistent_storage = init_persistent_storage();

    let ctx_action_storage = ContextActionStorage::new(&persistent_storage);
    let block_storage = BlockStorage::new(&persistent_storage);
    let blocks_iter = BlocksIterator::new(block_storage, &genesis_block_hash, blocks_limit);

    let (mut prev_cycle_commits, mut commits) = (vec![], vec![]);

    for block in blocks_iter {
        println!("applying block: {}", block.header.level());
        let merkle_rwlock = persistent_storage.merkle();
        let mut merkle = merkle_rwlock.write().unwrap();

        let mut actions = ctx_action_storage.get_by_block_hash(&block.hash).unwrap();
        actions.sort_by_key(|x| x.id);

        for action in actions.into_iter().map(|x| x.action) {
            // println!("applying action: ", action.)
            if let ContextAction::Commit { new_context_hash, .. } = &action {
                commits.push(new_context_hash[..].try_into().unwrap());
            }
            merkle.apply_context_action(&action).unwrap();
        }

        println!("applied block: {}", block.header.level());

        let (level, context_hash) = (block.header.level(), block.header.context());
        // let cycles = get_cycles_for_block(&persistent_storage, &context_hash);
        let cycles = 4096;

        if level % cycles == 0 && level > 0 && prev_cycle_commits.len() > 0 {
            println!("clearing previous cycle");
            for commit_hash in prev_cycle_commits.into_iter() {
                merkle.gc_commit(&commit_hash);
            }

            for commit_hash in commits.iter() {
                assert!(matches!(check_entry_hash(&merkle, commit_hash), Ok(_)));
            }

            prev_cycle_commits = commits;
            commits = vec![];
        }
    }
}