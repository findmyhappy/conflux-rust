// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use cfx_internal_common::state_root_with_aux_info::{
    StateRootAuxInfo, StateRootWithAuxInfo,
};
use cfx_statedb::{StateDb, StateDbExt};
use cfx_storage::{
    state_manager::{StateManager, StateManagerTrait},
    storage_db::{KeyValueDbTraitRead, SnapshotDbManagerTrait, SnapshotInfo},
    DeltaMptIterator, Error as StorageError, StateIndex, StorageConfiguration,
};
use cfx_types::{Address, AddressSpaceUtil, AddressWithSpace, H256};
use cfxcore::sync::Error;
use clap::{value_parser, Arg, ArgMatches, Command};
use log::LevelFilter;
use log4rs::{
    append::console::ConsoleAppender,
    config::{Appender, Config, Root},
};
use primitives::{
    Account, MerkleHash, StateRoot, StorageKey, StorageKeyWithSpace,
    MERKLE_NULL_NODE, NULL_EPOCH,
};
use std::{
    cmp::min, collections::HashMap, fs::remove_dir_all, path::PathBuf,
    str::FromStr, sync::Arc, time::Instant,
};

// cargo run --release -p cfxcore --example snapshot_merge_test
// cargo run --release -p cfxcore --example snapshot_merge_test -- --help
fn main() -> Result<(), Error> {
    enable_log();
    let matches = parse_args();

    // setup test directory
    let test_dir: &PathBuf = matches.get_one::<PathBuf>("test-dir").unwrap();
    if test_dir.exists() {
        remove_dir_all(test_dir)?;
    }

    // setup node 1
    println!("====================================================");
    println!("Setup node 1 ...");
    let state_manager =
        new_state_manager(test_dir.as_path().to_str().unwrap())?;
    let storage_manager = state_manager.get_storage_manager_arc();
    let snapshot_db_manager = state_manager
        .get_storage_manager()
        .get_snapshot_manager()
        .get_snapshot_db_manager();

    // state1 is only used to build a delta mpt, so the snapshot within it does
    // not matter.
    let mut accounts_map = HashMap::new();
    let (genesis_hash, _) = initialize_genesis(&state_manager)?;
    let accounts = matches.get_one::<usize>("accounts").unwrap().clone();
    let accounts_per_epoch = matches
        .get_one::<usize>("accounts-per-epoch")
        .unwrap()
        .clone();
    let state_root_1 = StateRootWithAuxInfo::genesis(&MERKLE_NULL_NODE);
    let mut height = 0;
    let (snapshot1_epoch, snapshot1_delta_root) = prepare_state(
        &state_manager,
        genesis_hash,
        &mut height,
        accounts,
        accounts_per_epoch,
        &mut accounts_map,
        &state_root_1,
        &state_root_1,
    )?;
    // Force other internal snapshot-related logic to be triggered
    height = storage_manager.get_snapshot_epoch_count() as u64;
    let delta_mpt = storage_manager
        .get_delta_mpt(&NULL_EPOCH)
        .expect("state exists");
    let delta_mpt_root = delta_mpt
        .get_root_node_ref(&snapshot1_delta_root)?
        .expect("root exists");
    let delta_mpt_iterator = DeltaMptIterator {
        mpt: delta_mpt,
        maybe_root_node: Some(delta_mpt_root),
    };

    let info = SnapshotInfo {
        height,
        parent_snapshot_height: 0,
        // This is unknown for now, and we don't care.
        merkle_root: Default::default(),
        parent_snapshot_epoch_id: NULL_EPOCH,
        pivot_chain_parts: vec![snapshot1_epoch],
        serve_one_step_sync: false,
        snapshot_info_kept_to_provide_sync: Default::default(),
    };
    let (mut snapshot_info_map_locked, snapshot_info1) = snapshot_db_manager
        .new_snapshot_by_merging(
            &NULL_EPOCH,
            snapshot1_epoch,
            delta_mpt_iterator,
            info,
            &storage_manager.snapshot_info_map_by_epoch,
            height,
            false,
        )?;
    storage_manager.register_new_snapshot(
        snapshot_info1.clone(),
        &mut snapshot_info_map_locked,
    )?;
    println!("After merging: {:?}", snapshot_info1);
    let state_root = StateRoot {
        snapshot_root: MERKLE_NULL_NODE,
        intermediate_delta_root: snapshot1_delta_root,
        delta_root: MERKLE_NULL_NODE,
    };
    let state_root_hash = state_root.compute_state_root_hash();
    let state_root_2 = StateRootWithAuxInfo {
        state_root,
        aux_info: StateRootAuxInfo {
            snapshot_epoch_id: NULL_EPOCH,
            intermediate_epoch_id: snapshot1_epoch,
            maybe_intermediate_mpt_key_padding: Some(
                StorageKeyWithSpace::delta_mpt_padding(
                    &MERKLE_NULL_NODE,
                    &MERKLE_NULL_NODE,
                ),
            ),
            delta_mpt_key_padding: StorageKeyWithSpace::delta_mpt_padding(
                &MERKLE_NULL_NODE,
                &snapshot1_delta_root,
            ),
            state_root_hash,
        },
    };
    let (snapshot2_epoch, snapshot2_delta_root) = prepare_state(
        &state_manager,
        snapshot1_epoch,
        &mut height,
        accounts,
        accounts_per_epoch,
        &mut accounts_map,
        &state_root_1,
        &state_root_2,
    )?;
    // Force other internal snapshot-related logic to be triggered
    height = 2 as u64 * storage_manager.get_snapshot_epoch_count() as u64;
    let delta_mpt = storage_manager
        .get_delta_mpt(&snapshot1_epoch)
        .expect("state exists");
    let delta_mpt_root = delta_mpt
        .get_root_node_ref(&snapshot2_delta_root)?
        .expect("root exists");
    let delta_mpt_iterator = DeltaMptIterator {
        mpt: delta_mpt,
        maybe_root_node: Some(delta_mpt_root),
    };
    let info = SnapshotInfo {
        height,
        parent_snapshot_height: snapshot_info1.height,
        // This is unknown for now, and we don't care.
        merkle_root: snapshot_info1.merkle_root,
        parent_snapshot_epoch_id: snapshot1_epoch,
        pivot_chain_parts: vec![snapshot2_epoch],
        serve_one_step_sync: false,
        snapshot_info_kept_to_provide_sync: Default::default(),
    };
    let (mut snapshot_info_map_locked, snapshot_info2) = snapshot_db_manager
        .new_snapshot_by_merging(
            &snapshot1_epoch,
            snapshot2_epoch,
            delta_mpt_iterator,
            info,
            &storage_manager.snapshot_info_map_by_epoch,
            height,
            false,
        )?;
    println!(
        "After merging: {:?}, accounts size {}",
        snapshot_info2,
        accounts_map.len()
    );
    storage_manager.register_new_snapshot(
        snapshot_info2.clone(),
        &mut snapshot_info_map_locked,
    )?;
    let snapshot2 = snapshot_db_manager
        .get_snapshot_by_epoch_id(
            &snapshot2_epoch,
            /* try_open = */ false,
            true,
        )?
        .expect("exists");
    for (addr, account) in &accounts_map {
        let value: Option<Box<[u8]>> =
            snapshot2.get(addr.address.as_bytes())?;
        assert!(value.is_some(), "Address {:?} does not exist", addr);
        let account_bytes = rlp::encode(account);
        let get_bytes = value.unwrap();
        assert_eq!(account_bytes.as_slice(), get_bytes.as_ref());
    }
    // TODO Make snapshot3 to compare the snapshot merkle_root
    let state_root_3 = StateRootWithAuxInfo::genesis(&MERKLE_NULL_NODE);
    height = 0;
    let (snapshot3_epoch, snapshot3_delta_root) = add_accounts(
        &state_manager,
        genesis_hash,
        &mut height,
        accounts_per_epoch,
        &accounts_map,
        &state_root_3,
        &state_root_3,
    )?;
    let delta_mpt = storage_manager
        .get_delta_mpt(&NULL_EPOCH)
        .expect("state exists");
    let delta_mpt_root = delta_mpt
        .get_root_node_ref(&snapshot3_delta_root)?
        .expect("root exists");
    let delta_mpt_iterator = DeltaMptIterator {
        mpt: delta_mpt,
        maybe_root_node: Some(delta_mpt_root),
    };

    let info = SnapshotInfo {
        height,
        parent_snapshot_height: 0,
        // This is unknown for now, and we don't care.
        merkle_root: Default::default(),
        parent_snapshot_epoch_id: NULL_EPOCH,
        pivot_chain_parts: vec![snapshot3_epoch],
        serve_one_step_sync: false,
        snapshot_info_kept_to_provide_sync: Default::default(),
    };
    let (mut snapshot_info_map_locked, snapshot_info3) = snapshot_db_manager
        .new_snapshot_by_merging(
            &NULL_EPOCH,
            snapshot3_epoch,
            delta_mpt_iterator,
            info,
            &storage_manager.snapshot_info_map_by_epoch,
            height,
            false,
        )?;
    storage_manager.register_new_snapshot(
        snapshot_info3.clone(),
        &mut snapshot_info_map_locked,
    )?;
    assert_eq!(snapshot_info3.merkle_root, snapshot_info2.merkle_root);
    Ok(())
}

fn parse_args<'a>() -> ArgMatches {
    Command::new("restore_checkpoint_delta")
        .arg(
            Arg::new("test-dir")
                .long("test-dir")
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help("Root directory for test")
                .default_value("test_restore_checkpoint_delta"),
        )
        .arg(
            Arg::new("accounts")
                .long("accounts")
                .value_name("NUM")
                .value_parser(value_parser!(usize))
                .help("Number of accounts in checkpoint")
                .default_value("10000"),
        )
        .arg(
            Arg::new("accounts-per-epoch")
                .long("accounts-per-epoch")
                .value_name("NUM")
                .value_parser(value_parser!(usize))
                .help("Number of accounts in each epoch")
                .default_value("1000"),
        )
        .arg(
            Arg::new("max-chunk-size")
                .long("max-chunk-size")
                .value_name("NUM")
                .value_parser(value_parser!(usize))
                .help("Maximum chunk size in bytes")
                .default_value("4000000"),
        )
        .get_matches()
}
fn new_state_manager(
    conflux_data_dir: &str,
) -> Result<Arc<StateManager>, Error> {
    let mut storage_conf = StorageConfiguration::new_default(
        conflux_data_dir,
        cfx_parameters::consensus::SNAPSHOT_EPOCHS_CAPACITY,
        cfx_parameters::consensus::ERA_DEFAULT_EPOCH_COUNT,
    );
    storage_conf.consensus_param.snapshot_epoch_count = 10000000;
    Ok(Arc::new(StateManager::new(storage_conf).unwrap()))
}

fn initialize_genesis(
    manager: &Arc<StateManager>,
) -> Result<(H256, MerkleHash), Error> {
    let mut state = manager.get_state_for_genesis_write();

    //    state.set(
    //        StorageKey::AccountKey(b"123"),
    //        vec![1, 2, 3].into_boxed_slice(),
    //    )?;
    //    state.set(
    //        StorageKey::AccountKey(b"124"),
    //        vec![1, 2, 4].into_boxed_slice(),
    //    )?;

    let root = state.compute_state_root()?;
    println!("genesis root: {:?}", root.state_root.delta_root);

    let genesis_hash = H256::from_str(
        "fa4e44bc69cca4cb2ae88a8fd452826faab9e8764e7eed934feede46c98962fa",
    )
    .unwrap();
    state.commit(genesis_hash.clone())?;

    Ok((genesis_hash, root.state_root.delta_root))
}

fn prepare_state(
    manager: &Arc<StateManager>, parent: H256, height: &mut u64,
    accounts: usize, accounts_per_epoch: usize,
    account_map: &mut HashMap<AddressWithSpace, Account>,
    old_state_root: &StateRootWithAuxInfo, state_root: &StateRootWithAuxInfo,
) -> Result<(H256, MerkleHash), StorageError> {
    let mut new_account_map = HashMap::new();
    for i in 0..accounts {
        let addr = Address::random().with_native_space();
        let account =
            Account::new_empty_with_balance(&addr, &i.into(), &0.into());
        new_account_map.insert(addr, account);
    }
    let r = add_accounts(
        manager,
        parent,
        height,
        accounts_per_epoch,
        &new_account_map,
        old_state_root,
        state_root,
    );
    account_map.extend(new_account_map.into_iter());
    r
}

fn add_accounts(
    manager: &Arc<StateManager>, parent: H256, height: &mut u64,
    accounts_per_epoch: usize,
    new_account_map: &HashMap<AddressWithSpace, Account>,
    old_state_root: &StateRootWithAuxInfo, state_root: &StateRootWithAuxInfo,
) -> Result<(H256, MerkleHash), StorageError> {
    let accounts = new_account_map.len();
    println!("begin to add {} accounts for snapshot...", accounts);
    let start = Instant::now();
    let mut epoch_id = parent;
    let mut pending = accounts;
    let mut account_iter = new_account_map.iter();
    while pending > 0 {
        let n = min(accounts_per_epoch, pending);
        let start2 = Instant::now();
        let state_root =
            if StateIndex::height_to_delta_height(
                *height,
                manager.get_storage_manager().get_snapshot_epoch_count(),
            ) == manager.get_storage_manager().get_snapshot_epoch_count()
            {
                old_state_root
            } else {
                state_root
            };
        let state_index = StateIndex::new_for_next_epoch(
            &epoch_id,
            state_root,
            *height,
            manager.get_storage_manager().get_snapshot_epoch_count(),
        );
        epoch_id =
            add_accounts_and_commit(manager, n, &mut account_iter, state_index);
        *height += 1;
        pending -= n;
        let progress = (accounts - pending) * 100 / accounts;
        println!(
            "{} accounts committed, progress = {}%, elapsed = {:?}",
            n,
            progress,
            start2.elapsed()
        );
    }

    println!("all accounts added in {:?}", start.elapsed());

    let root = manager
        // TODO consider snapshot.
        .get_state_no_commit(
            StateIndex::new_for_readonly(&epoch_id, state_root),
            /* try_open = */ false,
            None,
        )?
        .unwrap()
        .get_state_root()?
        .state_root
        .delta_root;
    println!("checkpoint: epoch_id={:?}, root: {:?}", epoch_id, root);
    Ok((epoch_id, root))
}

fn add_accounts_and_commit<'a, Iter>(
    manager: &Arc<StateManager>, accounts: usize, account_map: &mut Iter,
    state_index: StateIndex,
) -> H256
where
    Iter: Iterator<Item = (&'a AddressWithSpace, &'a Account)>,
{
    let state = manager
        .get_state_for_next_epoch(state_index, false)
        .unwrap()
        .unwrap();
    let mut state = StateDb::new(state);
    for _ in 0..accounts {
        let (addr, account) =
            account_map.next().expect("Caller has checked the size");
        state
            .set(
                StorageKey::new_account_key(&addr.address)
                    .with_space(addr.space),
                account,
                None,
            )
            .unwrap();
    }
    let epoch = H256::random();
    state.commit(epoch, /* debug_record = */ None).unwrap();
    epoch
}

fn enable_log() {
    let stdout = ConsoleAppender::builder().build();
    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .build(Root::builder().appender("stdout").build(LevelFilter::Warn))
        .unwrap();
    log4rs::init_config(config).expect("success");
}
