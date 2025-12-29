use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use dns_lookup::lookup_host;
use parking_lot::Mutex;
use pea2pea::{
    protocols::{Handshake, Reading, Writing},
    Pea2Pea,
};
use rand::prelude::IteratorRandom;
use tokio::{signal, time::sleep};
use tracing::{debug, error, info, warn};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use ziggurat_core_crawler::summary::NetworkSummary;
use ziggurat_zcash::wait_until;

use crate::{
    metrics::{NetworkMetrics, ZCASH_P2P_DEFAULT_MAINNET_PORT},
    network::{ConnectionState, KnownNode},
    protocol::{
        Crawler, MAIN_LOOP_INTERVAL_SECS, MAX_WAIT_FOR_ADDR_SECS, NUM_CONN_ATTEMPTS_PERIODIC,
        RECONNECT_INTERVAL_SECS,
    },
    rpc::{initialize_rpc_server, RpcContext},
};

mod metrics;
mod network;
mod protocol;
mod rpc;

const SEED_WAIT_LOOP_INTERVAL_MS: u64 = 500;
const SEED_RESPONSE_TIMEOUT_MS: u64 = 120_000;
const SUMMARY_LOOP_INTERVAL: u64 = 30;
const LOG_PATH: &str = "crawler-log.txt";

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser, num_args(1..), required = true)]
    seed_addrs: Vec<String>,

    #[clap(short, long, value_parser, default_value_t = MAIN_LOOP_INTERVAL_SECS)]
    crawl_interval: u64,

    #[clap(short, long, value_parser)]
    rpc_addr: Option<SocketAddr>,

    #[clap(short, long, value_parser, default_value_t = ZCASH_P2P_DEFAULT_MAINNET_PORT)]
    node_listening_port: u16,
}

fn start_logger(default_level: LevelFilter) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter
            .add_directive("tokio_util=off".parse().unwrap())
            .add_directive("mio=off".parse().unwrap()),
        _ => EnvFilter::default()
            .add_directive(default_level.into())
            .add_directive("tokio_util=off".parse().unwrap())
            .add_directive("mio=off".parse().unwrap()),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn parse_addrs(seed_addrs: Vec<String>, node_listening_port: u16) -> Vec<SocketAddr> {
    let mut parsed_addrs = Vec::with_capacity(seed_addrs.len());

    for seed_addr in seed_addrs {
        if let Ok(addr) = seed_addr.parse::<SocketAddr>() {
            parsed_addrs.push(addr);
            continue;
        }
        if let Ok(addr) = seed_addr.parse::<IpAddr>() {
            parsed_addrs.push(SocketAddr::new(addr, node_listening_port));
            println!("no port specified for address: {}, using default: {}", seed_addr, node_listening_port);
            continue;
        }
        let mut clean_addrs = seed_addr.clone();
        let mut addr_split: Vec<_> = seed_addr.split(":").collect();
        let mut port = node_listening_port;
        if addr_split.len() > 1 {
            if let Some(p) = addr_split.pop() {
                port = p.parse().unwrap();
            }
            clean_addrs = addr_split.into_iter().collect();
        }
        let response = lookup_host(&clean_addrs);
        if let Ok(response) = response {
            for address in response.iter() {
                parsed_addrs.push(SocketAddr::new(*address, port));
                println!("DNS seed {} address added: {}", seed_addr, address);
            }
        } else {
            error!("failed to resolve address: {}", seed_addr);
        }
    }

    parsed_addrs
}

#[tokio::main]
async fn main() {
    start_logger(LevelFilter::INFO);
    let args = Args::parse();
    let seed_addrs = parse_addrs(args.seed_addrs, args.node_listening_port);

    let crawler = Crawler::new().await;

    let mut network_metrics = NetworkMetrics::default();
    let summary_snapshot = Arc::new(Mutex::new(NetworkSummary::default()));
    let nodes_snapshot: Arc<Mutex<HashMap<SocketAddr, KnownNode>>> = Arc::new(Mutex::new(HashMap::new()));

    let _rpc_handle = if let Some(addr) = args.rpc_addr {
        let rpc_context = RpcContext::new(Arc::clone(&summary_snapshot), Arc::clone(&nodes_snapshot));
        let rpc_handle = initialize_rpc_server(addr, rpc_context).await;
        Some(rpc_handle)
    } else {
        None
    };

    crawler.enable_handshake().await;
    crawler.enable_reading().await;
    crawler.enable_writing().await;

    for addr in &seed_addrs {
        let crawler_clone = crawler.clone();
        let addr = *addr;

        tokio::spawn(async move {
            crawler_clone
                .known_network
                .nodes
                .write()
                .insert(addr, KnownNode::default());
            let _ = crawler_clone.connect(addr).await;
        });
    }

    wait_until!(Duration::from_secs(3), crawler.node().num_connected() >= 1);

    wait_until!(
        Duration::from_millis(SEED_RESPONSE_TIMEOUT_MS),
        crawler.known_network.nodes().len() > seed_addrs.len(),
        Duration::from_millis(SEED_WAIT_LOOP_INTERVAL_MS)
    );

    let crawler_clone = crawler.clone();
    let crawling_loop_task = tokio::spawn(async move {
        let crawler = crawler_clone;
        loop {
            info!(parent: crawler.node().span(), "asking peers for their peers (connected to {})", crawler.node().num_connected());
            info!(parent: crawler.node().span(), "known addrs: {}", crawler.known_network.num_nodes());

            for (addr, _) in crawler
                .known_network
                .nodes()
                .into_iter()
                .filter(|(_, node)| {
                    if node.state == ConnectionState::Connected {
                        if let Some(i) = node.last_connected {
                            i.elapsed().as_secs() >= MAX_WAIT_FOR_ADDR_SECS
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                })
            {
                warn!(parent: crawler.node().span(), "disconnecting from node {} because it didn't send us proper addr message", addr);
                crawler.node().disconnect(addr).await;
                crawler
                    .known_network
                    .set_node_state(addr, ConnectionState::Disconnected);
            }

            for (addr, _) in crawler
                .known_network
                .nodes()
                .into_iter()
                .filter(|(_, node)| {
                    if let Some(i) = node.last_connected {
                        i.elapsed().as_secs() >= RECONNECT_INTERVAL_SECS
                    } else {
                        true
                    }
                })
                .choose_multiple(&mut rand::thread_rng(), NUM_CONN_ATTEMPTS_PERIODIC)
            {
                if crawler.should_connect(addr) {
                    let crawler_clone = crawler.clone();
                    tokio::spawn(async move {
                        let _ = crawler_clone.connect(addr).await;
                    });
                }
            }

            sleep(Duration::from_secs(args.crawl_interval)).await;
        }
    });

    let crawler_clone = crawler.clone();
    let summary = Arc::clone(&summary_snapshot);
    let nodes_snap = Arc::clone(&nodes_snapshot);

    thread::spawn(move || {
        loop {
            let start_time = Instant::now();

            if crawler.known_network.num_connections() > 0 {
                crawler.known_network.remove_old_connections();
                network_metrics.update_graph(&crawler);
                let new_summary = network_metrics.request_summary(&crawler);
                *summary_snapshot.lock() = new_summary;
            }

            // Update nodes snapshot for RPC
            *nodes_snap.lock() = crawler.known_network.nodes();

            let delta_time =
                Duration::from_secs(SUMMARY_LOOP_INTERVAL).saturating_sub(start_time.elapsed());

            if delta_time.is_zero() {
                warn!(parent: crawler.node().span(), "summary calculation took more time than the loop interval");
            }
            info!(parent: crawler.node().span(), "summary calculation took: {:?}", start_time.elapsed());

            thread::sleep(delta_time);
        }
    });

    let _ = signal::ctrl_c().await;
    debug!(parent: crawler_clone.node().span(), "interrupt received, exiting process");

    crawling_loop_task.abort();
    let _ = crawling_loop_task.await;
    crawler_clone.node().shut_down().await;

    let summary = summary.lock();
    info!(parent: crawler_clone.node().span(), "{}", summary);
    if let Err(e) = summary.log_to_file(LOG_PATH) {
        error!(parent: crawler_clone.node().span(), "couldn't write summary to file: {}", e);
    }
}
