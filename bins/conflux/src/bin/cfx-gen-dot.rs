// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use cfx_types::{hexstr_to_h256, H256};
use primitives::Block;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

fn open_db(db_path: &str) -> std::io::Result<Arc<db::SystemDB>> {
    let db_config = db::db_config(
        std::path::Path::new(db_path),
        None,
        db::DatabaseCompactionProfile::default(),
        cfxcore::db::NUM_COLUMNS,
        false,
    );

    db::open_database(db_path, &db_config)
}

fn retrieve_block(db: &Arc<db::SystemDB>, hash: &H256) -> Option<Block> {
    let block = db.key_value().get(cfxcore::db::COL_BLOCKS, hash.as_bytes()).expect(
        "Low level database error when fetching block. Some issue with disk?",
    )?;

    let rlp = rlp::Rlp::new(&block);
    let block =
        Block::decode_with_tx_public(&rlp).expect("Wrong block rlp format!");

    Some(block)
}

fn fmt_hash(hash: &H256) -> String {
    format!("{:?}", hash)[0..14].to_string() + "..."
}

fn print_edge(from: &H256, to: &H256) {
    println!("\"{}\" -> \"{}\";", fmt_hash(from), fmt_hash(to));
}

fn print_ref_edge(from: &H256, to: &H256) {
    println!(
        "\"{}\" -> \"{}\" [style=dotted];",
        fmt_hash(from),
        fmt_hash(to)
    );
}

fn print_graph(db: &Arc<db::SystemDB>, from: &H256, max_depth: u32) {
    println!("digraph G {{");
    println!("rankdir=\"RL\";");
    println!("node [shape=box];");

    let mut queue: VecDeque<(u32, H256)> = VecDeque::new();
    let mut visited: HashSet<H256> = HashSet::new();
    queue.push_back((0, from.clone()));

    while let Some((depth, hash)) = queue.pop_front() {
        if visited.contains(&hash) || depth == max_depth {
            continue;
        }

        assert!(depth < max_depth);
        visited.insert(hash);

        if let Some(block) = retrieve_block(&db, &hash) {
            let parent = block.block_header.parent_hash();
            let refs = block.block_header.referee_hashes();

            print_edge(&hash, &parent);
            queue.push_back((depth + 1, *parent));

            for r in refs {
                print_ref_edge(&hash, &r);
                queue.push_back((depth + 1, *r));
            }
        }
    }

    println!("}}");
}

struct Config {
    db_path: String,
    from_block: H256,
    max_depth: u32,
}
fn parse_config() -> Config {
    let matches = clap::Command::new("cfx-gen-dot")
        .version("0.1")
        .about(
"Generate Graphviz dot files from your local blockchain db
Example usage:
    cfx-gen-dot
        --db-path ./run/blockchain_db
        --from-block 0x3159d8d9b125a738cc226a9b85f6d7fa0da1567018c6771f9bf658e83496834d
        --max-depth 10000
        > graph.dot
    dot -Tsvg graph.dot -o graph.svg")
        .arg(
            clap::Arg::new("db-path")
                .long("db-path")
                .value_name("PATH")
                .help("Specifies local blockchain db directory")
                .required(true),
        )
        .arg(
            clap::Arg::new("from-block")
                .long("from-block")
                .value_name("HASH")
                .help("Sets starting block of DAG traversal")
                .required(true),
        )
        .arg(
            clap::Arg::new("max-depth")
                .long("max-depth")
                .value_name("NUM")
                .value_parser(clap::value_parser!(u32))
                .help("Sets maximum depth for traversal")
                .required(true)
        )
        .get_matches();

    let db_path = matches.get_one::<String>("db-path").unwrap();
    let max_depth = matches.get_one::<u32>("max-depth").unwrap().clone();

    let from_block = {
        let mut from =
            matches.get_one::<String>("from-block").unwrap().as_str();

        if from.starts_with("0x") {
            from = &from[2..];
        }

        hexstr_to_h256(from)
    };

    Config {
        db_path: String::from(db_path),
        from_block,
        max_depth,
    }
}

fn main() {
    let config = parse_config();
    let db = open_db(&config.db_path).unwrap();
    print_graph(&db, &config.from_block, config.max_depth);
}
