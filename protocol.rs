// p2p protocol stuff - handshake, message handling

use std::{io, net::SocketAddr, sync::Arc, time::Instant};
use futures_util::SinkExt;
use pea2pea::{protocols::{Handshake, Reading, Writing}, Config, Connection, ConnectionSide, Node as Pea2PeaNode, Pea2Pea};
use tokio_util::codec::Framed;
use tracing::*;
use ziggurat_zcash::{protocol::{message::Message, payload::{block::Headers, Addr, VarStr, Version}}, tools::synthetic_node::MessageCodec};
use super::network::KnownNetwork;
use crate::network::ConnectionState;

pub const NUM_CONN_ATTEMPTS_PERIODIC: usize = 2000;
pub const MAX_CONCURRENT_CONNECTIONS: u16 = 3500;
pub const RECONNECT_INTERVAL_SECS: u64 = 45;
pub const MAX_WAIT_FOR_ADDR_SECS: u64 = 90;

#[derive(Clone)]
pub struct Crawler {
    node: Pea2PeaNode,
    pub known_network: Arc<KnownNetwork>,
    pub start_time: Instant,
}

impl Pea2Pea for Crawler {
    fn node(&self) -> &Pea2PeaNode { &self.node }
}

impl Crawler {
    pub async fn new() -> Self {
        let cfg = Config { name: Some("crawler".into()), listener_ip: None, max_connections: MAX_CONCURRENT_CONNECTIONS, ..Default::default() };
        Self { node: Pea2PeaNode::new(cfg), known_network: Default::default(), start_time: Instant::now() }
    }

    pub async fn connect(&self, addr: SocketAddr) -> io::Result<()> {
        trace!(parent: self.node().span(), "connecting to {}", addr);
        let ts = Instant::now();
        let res = self.node.connect(addr).await;
        if let Some(n) = self.known_network.nodes.write().get_mut(&addr) {
            match res {
                Ok(_) => { n.connection_failures = 0; n.last_connected = Some(ts); n.handshake_time = Some(ts.elapsed()); n.state = ConnectionState::Connected; }
                Err(_) => { n.connection_failures += 1; }
            }
        }
        res
    }

    pub fn should_connect(&self, addr: SocketAddr) -> bool {
        if self.known_network.nodes().get(&addr).is_none() { return false; }
        if self.node().num_connected() + self.node().num_connecting() >= MAX_CONCURRENT_CONNECTIONS.into() { return false; }
        if self.node().is_connected(addr) || self.node().is_connecting(addr) { return false; }
        true
    }
}

#[async_trait::async_trait]
impl Handshake for Crawler {
    const TIMEOUT_MS: u64 = 2000;

    async fn perform_handshake(&self, mut conn: Connection) -> io::Result<Connection> {
        let addr = conn.addr();
        let listen: SocketAddr = ([127,0,0,1], 0).into();
        let mut stream = Framed::new(self.borrow_stream(&mut conn), MessageCodec::default());

        // pretend to be zcashd 5.4.2
        let mut ver = Version::new(addr, listen);
        ver.user_agent = VarStr("/MagicBean:5.4.2/".into());
        ver.start_height = 3_150_000;
        ver.relay = true;
        stream.send(Message::Version(ver)).await?;
        Ok(conn)
    }
}

#[async_trait::async_trait]
impl Reading for Crawler {
    type Message = Message;
    type Codec = MessageCodec;
    fn codec(&self, _: SocketAddr, _: ConnectionSide) -> Self::Codec { Default::default() }

    async fn process_message(&self, src: SocketAddr, msg: Self::Message) -> io::Result<()> {
        match msg {
            Message::Addr(a) => {
                let n = a.addrs.len();
                info!(parent: self.node().span(), "got {} addrs from {}", n, src);
                let addrs: Vec<_> = a.addrs.iter().map(|x| x.addr).collect();
                self.known_network.add_addrs(src, &addrs);
                // disconnect after getting addrs (unless its just echoing our addr back)
                if n > 1 || (n == 1 && a.addrs[0].addr != src) {
                    self.node().disconnect(src).await;
                    self.known_network.set_node_state(src, ConnectionState::Disconnected);
                }
            }
            Message::Ping(nonce) => { let _ = self.unicast(src, Message::Pong(nonce))?.await; }
            Message::GetAddr => { let _ = self.unicast(src, Message::Addr(Addr::empty()))?.await; }
            Message::GetHeaders(_) => { let _ = self.unicast(src, Message::Headers(Headers::empty()))?.await; }
            Message::GetData(inv) => { let _ = self.unicast(src, Message::NotFound(inv.clone()))?.await; }
            Message::Version(v) => {
                info!(parent: self.node().span(), "version from {}", src);
                if let Some(n) = self.known_network.nodes.write().get_mut(&src) {
                    n.protocol_version = Some(v.version);
                    n.user_agent = Some(v.user_agent);
                    n.services = Some(v.services);
                    n.start_height = Some(v.start_height);
                }
                let _ = self.unicast(src, Message::Verack)?.await;
                let _ = self.unicast(src, Message::GetAddr)?.await;
            }
            _ => {}
        }
        Ok(())
    }
}

impl Writing for Crawler {
    type Message = Message;
    type Codec = MessageCodec;
    fn codec(&self, _: SocketAddr, _: ConnectionSide) -> Self::Codec { Default::default() }
}
