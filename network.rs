// network state - keeps track of nodes we know about

use std::{collections::{HashMap, HashSet}, net::SocketAddr, time::{Duration, Instant}};
use parking_lot::RwLock;
use ziggurat_core_crawler::connection::KnownConnection;
use ziggurat_zcash::protocol::payload::{ProtocolVersion, VarStr};

pub const LAST_SEEN_CUTOFF: u64 = 600; // 10 min

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum ConnectionState { #[default] Disconnected, Connected }

#[derive(Debug, Default, Clone)]
pub struct KnownNode {
    pub last_connected: Option<Instant>,
    pub handshake_time: Option<Duration>,
    pub protocol_version: Option<ProtocolVersion>,
    pub user_agent: Option<VarStr>,
    pub start_height: Option<i32>,
    pub services: Option<u64>,
    pub connection_failures: u8,
    pub state: ConnectionState,
}

#[derive(Default)]
pub struct KnownNetwork {
    pub nodes: RwLock<HashMap<SocketAddr, KnownNode>>,
    pub connections: RwLock<HashSet<KnownConnection>>,
}

impl KnownNetwork {
    pub fn add_addrs(&self, src: SocketAddr, addrs: &[SocketAddr]) {
        { let mut c = self.connections.write(); for a in addrs { c.insert(KnownConnection::new(src, *a)); } }
        let mut n = self.nodes.write();
        n.entry(src).or_default();
        for a in addrs { n.entry(*a).or_default(); }
    }

    pub fn set_node_state(&self, addr: SocketAddr, state: ConnectionState) {
        if let Some(n) = self.nodes.write().get_mut(&addr) { n.state = state; }
    }

    pub fn connections(&self) -> HashSet<KnownConnection> { self.connections.read().clone() }
    pub fn nodes(&self) -> HashMap<SocketAddr, KnownNode> { self.nodes.read().clone() }
    #[allow(dead_code)]
    pub fn num_connections(&self) -> usize { self.connections.read().len() }
    pub fn num_nodes(&self) -> usize { self.nodes.read().len() }

    pub fn remove_old_connections(&self) {
        let old: Vec<_> = self.connections().into_iter().filter(|c| c.last_seen.elapsed().as_secs() > LAST_SEEN_CUTOFF).collect();
        if !old.is_empty() {
            let mut c = self.connections.write();
            for x in old { c.remove(&x); }
        }
    }
}
