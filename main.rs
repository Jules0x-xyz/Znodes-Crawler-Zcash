// znodes - zcash network crawler
// connects to dns seeds, grabs peer addresses, tracks whos online

use std::{
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
use rand::seq::SliceRandom;
use tokio::{signal, time::sleep};
use tracing::{error, info};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use ziggurat_core_crawler::summary::NetworkSummary;

use crate::{
    metrics::{NetworkMetrics, ZCASH_P2P_DEFAULT_MAINNET_PORT},
    network::{ConnectionState, KnownNode},
    protocol::{Crawler, MAX_WAIT_FOR_ADDR_SECS, NUM_CONN_ATTEMPTS_PERIODIC, RECONNECT_INTERVAL_SECS},
    rpc::{initialize_rpc_server, RpcContext},
};

mod metrics;
mod network;
mod protocol;
mod rpc;

const SEED_WAIT_INTERVAL: u64 = 500;
const SEED_TIMEOUT: u64 = 120_000;
const DNS_REFRESH: u64 = 120;
const SUMMARY_INTERVAL: u64 = 60;
const LOG_FILE: &str = "crawler-log.txt";

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser, num_args(1..), required = true)]
    seed_addrs: Vec<String>,
    #[clap(short, long, value_parser, default_value_t = 10)]
    crawl_interval: u64,
    #[clap(short, long, value_parser)]
    rpc_addr: Option<SocketAddr>,
    #[clap(short, long, value_parser, default_value_t = ZCASH_P2P_DEFAULT_MAINNET_PORT)]
    node_listening_port: u16,
}

fn setup_logging(level: LevelFilter) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(f) => f.add_directive("tokio_util=off".parse().unwrap()).add_directive("mio=off".parse().unwrap()),
        _ => EnvFilter::default().add_directive(level.into()).add_directive("tokio_util=off".parse().unwrap()).add_directive("mio=off".parse().unwrap()),
    };
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).init();
}

fn parse_addrs(addrs: Vec<String>, default_port: u16) -> Vec<SocketAddr> {
    let mut out = Vec::with_capacity(addrs.len());
    for addr in addrs {
        if let Ok(sa) = addr.parse::<SocketAddr>() { out.push(sa); continue; }
        if let Ok(ip) = addr.parse::<IpAddr>() {
            out.push(SocketAddr::new(ip, default_port));
            println!("no port for {}, using {}", addr, default_port);
            continue;
        }
        let mut clean = addr.clone();
        let parts: Vec<_> = addr.split(':').collect();
        let mut port = default_port;
        if parts.len() > 1 {
            if let Ok(p) = parts.last().unwrap().parse::<u16>() {
                port = p;
                clean = parts[..parts.len()-1].join("");
            }
        }
        match lookup_host(&clean) {
            Ok(ips) => { for ip in ips { out.push(SocketAddr::new(ip, port)); println!("dns {} -> {}", addr, ip); } }
            Err(_) => error!("cant resolve: {}", addr),
        }
    }
    out
}

#[tokio::main]
async fn main() {
    setup_logging(LevelFilter::INFO);
    let args = Args::parse();
    let seeds = args.seed_addrs.clone();
    let addrs = parse_addrs(seeds.clone(), args.node_listening_port);
    if addrs.is_empty() { error!("no valid seeds"); return; }

    let crawler = Crawler::new().await;
    let mut metrics = NetworkMetrics::default();
    let summary = Arc::new(Mutex::new(NetworkSummary::default()));
    let nodes_snap = Arc::new(Mutex::new(std::collections::HashMap::new()));

    let _rpc = if let Some(addr) = args.rpc_addr {
        Some(initialize_rpc_server(addr, RpcContext::new(Arc::clone(&summary), Arc::clone(&nodes_snap))).await)
    } else { None };

    crawler.enable_handshake().await;
    crawler.enable_reading().await;
    crawler.enable_writing().await;

    for addr in &addrs {
        let c = crawler.clone();
        let a = *addr;
        tokio::spawn(async move { c.known_network.nodes.write().insert(a, KnownNode::default()); let _ = c.connect(a).await; });
    }

    // dns refresh task
    { let c = crawler.clone(); let s = seeds.clone(); let p = args.node_listening_port;
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(DNS_REFRESH)).await;
            for seed in &s {
                let resolved = parse_addrs(vec![seed.clone()], p);
                for addr in resolved {
                    let mut new = false;
                    { let mut nodes = c.known_network.nodes.write(); if !nodes.contains_key(&addr) { nodes.insert(addr, KnownNode::default()); new = true; } }
                    if new { let cc = c.clone(); tokio::spawn(async move { let _ = cc.connect(addr).await; }); }
                }
            }
        }
    }); }

    info!("waiting for connection...");
    for _ in 0..30 { if crawler.node().num_connected() >= 1 { break; } sleep(Duration::from_millis(500)).await; }

    info!("waiting for addrs...");
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(SEED_TIMEOUT) {
        if crawler.known_network.nodes().len() > addrs.len() { info!("got addrs"); break; }
        sleep(Duration::from_millis(SEED_WAIT_INTERVAL)).await;
    }

    let c = crawler.clone();
    let crawl_task = tokio::spawn(async move {
        loop {
            info!(parent: c.node().span(), "crawling - conn:{} known:{}", c.node().num_connected(), c.known_network.num_nodes());

            for (addr, _) in c.known_network.nodes().into_iter().filter(|(_, n)| {
                n.state == ConnectionState::Connected && n.last_connected.map_or(true, |t| t.elapsed().as_secs() >= MAX_WAIT_FOR_ADDR_SECS)
            }) { c.node().disconnect(addr).await; c.known_network.set_node_state(addr, ConnectionState::Disconnected); }

            let snap = c.known_network.nodes();
            let mut need_info = Vec::new();
            let mut have_info = Vec::new();
            for (addr, node) in snap.into_iter() {
                if node.last_connected.map_or(false, |t| t.elapsed().as_secs() < RECONNECT_INTERVAL_SECS) { continue; }
                if node.user_agent.is_none() { need_info.push(addr); } else { have_info.push(addr); }
            }
            {
                let mut rng = rand::thread_rng();
                need_info.shuffle(&mut rng);
                have_info.shuffle(&mut rng);
            }

            let mut targets = Vec::with_capacity(NUM_CONN_ATTEMPTS_PERIODIC);
            targets.extend(need_info.into_iter().take(NUM_CONN_ATTEMPTS_PERIODIC));
            targets.extend(have_info.into_iter().take(NUM_CONN_ATTEMPTS_PERIODIC.saturating_sub(targets.len())));

            for addr in targets { if c.should_connect(addr) { let cc = c.clone(); tokio::spawn(async move { let _ = cc.connect(addr).await; }); } }
            sleep(Duration::from_secs(args.crawl_interval)).await;
        }
    });

    let c2 = crawler.clone();
    let sum = Arc::clone(&summary);
    let nsnap = Arc::clone(&nodes_snap);
    thread::spawn(move || {
        loop {
            let t = Instant::now();
            c2.known_network.remove_old_connections();
            metrics.update_graph(&c2);
            *sum.lock() = metrics.request_summary(&c2);
            *nsnap.lock() = c2.known_network.nodes();
            thread::sleep(Duration::from_secs(SUMMARY_INTERVAL).saturating_sub(t.elapsed()));
        }
    });

    let _ = signal::ctrl_c().await;
    crawl_task.abort();
    let _ = crawl_task.await;
    crawler.node().shut_down().await;
    let s = summary.lock();
    info!(parent: crawler.node().span(), "{}", s);
    let _ = s.log_to_file(LOG_FILE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    #[test]
    fn test_parse() {
        let r = parse_addrs(vec!["127.0.0.1".into()], 8233);
        assert_eq!(r[0], SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127,0,0,1)), 8233));
    }
}
