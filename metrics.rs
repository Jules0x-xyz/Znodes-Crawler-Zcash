// metrics - graph stuff and network summary

use std::{collections::HashMap, net::SocketAddr};
use regex::Regex;
use spectre::{edge::Edge, graph::Graph};
use ziggurat_core_crawler::summary::{NetworkSummary, NetworkType};
use crate::{network::{KnownNode, LAST_SEEN_CUTOFF}, Crawler};

const MIN_HEIGHT: i32 = 2_000_000;
pub const ZCASH_P2P_DEFAULT_MAINNET_PORT: u16 = 8233;
pub const ZCASH_P2P_DEFAULT_TESTNET_PORT: u16 = 18233;

#[derive(Default)]
pub struct NetworkMetrics { graph: Graph<SocketAddr> }

impl NetworkMetrics {
    pub fn update_graph(&mut self, crawler: &Crawler) {
        for c in crawler.known_network.connections() {
            let e = Edge::new(c.a, c.b);
            if c.last_seen.elapsed().as_secs() > LAST_SEEN_CUTOFF { self.graph.remove(&e); }
            else { self.graph.insert(e); }
        }
    }

    pub fn request_summary(&mut self, crawler: &Crawler) -> NetworkSummary {
        build_summary(crawler, &self.graph)
    }
}

fn classify_nodes(nodes: &HashMap<SocketAddr, KnownNode>, good: &[SocketAddr]) -> Vec<NetworkType> {
    let zc_re = Regex::new(r"^/MagicBean:(\d)\.(\d)\.(\d)/$").unwrap();
    let zb_re = Regex::new(r"^/Zebra:(\d)\.(\d)\.(\d)").unwrap();

    good.iter().map(|addr| {
        let n = &nodes[addr];
        let agent = n.user_agent.as_ref().map(|x| x.0.clone()).unwrap_or_default();
        let port_ok = addr.port() == ZCASH_P2P_DEFAULT_MAINNET_PORT || addr.port() == ZCASH_P2P_DEFAULT_TESTNET_PORT;

        // check zcashd
        let mut agent_ok = false;
        if let Some(cap) = zc_re.captures(&agent) {
            let major: u32 = cap.get(1).unwrap().as_str().parse().unwrap_or(0);
            if major >= 6 { return NetworkType::Unknown; } // flux
            agent_ok = true;
        }
        if zb_re.is_match(&agent) { agent_ok = true; }

        let h = n.start_height.unwrap_or(0);
        if h < MIN_HEIGHT { return NetworkType::Unknown; }
        if port_ok || agent_ok { NetworkType::Zcash } else { NetworkType::Unknown }
    }).collect()
}

fn build_summary(crawler: &Crawler, graph: &Graph<SocketAddr>) -> NetworkSummary {
    let nodes = crawler.known_network.nodes();
    let conns = crawler.known_network.connections();

    let good: Vec<_> = nodes.iter().filter_map(|(a, n)| n.last_connected.map(|_| *a)).collect();

    let mut versions = HashMap::new();
    let mut agents = HashMap::new();
    for n in nodes.values() {
        if let Some(v) = n.protocol_version {
            *versions.entry(v.0).or_insert(0) += 1;
            if let Some(ref ua) = n.user_agent { *agents.entry(ua.0.clone()).or_insert(0) += 1; }
        }
    }

    let types = classify_nodes(&nodes, &good);

    // build adjacency manually
    let indices: Vec<Vec<usize>> = good.iter().enumerate().map(|(_, a)| {
        good.iter().enumerate().filter_map(|(j, b)| {
            if a != b && graph.contains(&Edge::new(*a, *b)) { Some(j) } else { None }
        }).collect()
    }).collect();

    NetworkSummary {
        num_known_nodes: nodes.len(),
        num_good_nodes: good.len(),
        num_known_connections: conns.len(),
        num_versions: versions.values().sum(),
        protocol_versions: versions,
        user_agents: agents,
        crawler_runtime: crawler.start_time.elapsed(),
        node_addrs: good,
        node_network_types: types,
        nodes_indices: indices,
    }
}
