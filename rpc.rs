// rpc server - json-rpc api for getting node info

use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use jsonrpsee::server::{RpcModule, ServerBuilder, ServerHandle};
use parking_lot::Mutex;
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::debug;
use ziggurat_core_crawler::summary::NetworkSummary;
use crate::network::KnownNode;

const HEIGHT_TOLERANCE: i32 = 10000;
const MIN_HEIGHT: i32 = 2_500_000;
pub const MAX_RESPONSE_SIZE: u32 = 200_000_000;

#[derive(Clone, Serialize)]
pub struct NodeInfo {
    pub ip: String, pub port: u16,
    pub protocol_version: Option<u32>, pub user_agent: Option<String>,
    pub height: Option<i32>, pub services: Option<u64>,
    pub last_seen_secs: u64, pub is_relevant: bool, pub is_flux: bool, pub client_type: String,
}

#[derive(Clone, Serialize)]
pub struct Stats {
    pub num_known_nodes: usize, pub num_contacted_nodes: usize, pub num_relevant_zcash_nodes: usize,
    pub num_zcashd_nodes: usize, pub num_zebra_nodes: usize, pub num_flux_nodes: usize, pub num_other_nodes: usize,
    pub tip_height_estimate: i32, pub crawler_runtime_secs: u64,
}

#[derive(Clone, Serialize)]
pub struct NodesResponse { pub stats: Stats, pub nodes: Vec<NodeInfo> }

pub struct RpcContext {
    summary: Arc<Mutex<NetworkSummary>>,
    nodes: Arc<Mutex<HashMap<SocketAddr, KnownNode>>>,
}

impl RpcContext {
    pub fn new(s: Arc<Mutex<NetworkSummary>>, n: Arc<Mutex<HashMap<SocketAddr, KnownNode>>>) -> Self {
        Self { summary: s, nodes: n }
    }
}

fn get_tip(nodes: &HashMap<SocketAddr, KnownNode>) -> i32 {
    let mut h: Vec<_> = nodes.values().filter_map(|n| n.start_height).filter(|x| *x > MIN_HEIGHT).collect();
    if h.is_empty() { return MIN_HEIGHT; }
    h.sort();
    h.get((h.len() as f64 * 0.95) as usize).copied().unwrap_or(MIN_HEIGHT)
}

fn client_type(ua: &str) -> (String, bool) {
    let lower = ua.to_lowercase();
    if lower.contains("flux") { return ("flux".into(), true); }
    if lower.contains("magicbean") {
        if let Some(i) = lower.find("magicbean:") {
            let ver = &lower[i+10..];
            if let Some(d) = ver.find('.') {
                if let Ok(maj) = ver[..d].parse::<u32>() { if maj >= 6 { return ("flux".into(), true); } }
            }
        }
        return ("zcashd".into(), false);
    }
    if lower.contains("zebra") { return ("zebra".into(), false); }
    ("other".into(), false)
}

fn is_good_node(n: &KnownNode, tip: i32) -> bool {
    let ua = match &n.user_agent { Some(x) => x.0.to_lowercase(), None => return false };
    if !ua.starts_with("/magicbean") && !ua.starts_with("/zebra") { return false; }
    if ua.contains("flux") { return false; }
    if ua.contains("magicbean") {
        if let Some(i) = ua.find("magicbean:") {
            let ver = &ua[i+10..];
            if let Some(d) = ver.find('.') {
                if let Ok(maj) = ver[..d].parse::<u32>() { if maj >= 6 { return false; } }
            }
        }
    }
    let h = n.start_height.unwrap_or(0);
    if h < MIN_HEIGHT { return false; }
    if !ua.contains("magicbean") && (h - tip).abs() > HEIGHT_TOLERANCE { return false; }
    true
}

pub async fn initialize_rpc_server(addr: SocketAddr, ctx: RpcContext) -> ServerHandle {
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
    let mw = tower::ServiceBuilder::new().layer(cors);
    let srv = ServerBuilder::default().set_middleware(mw).max_response_body_size(MAX_RESPONSE_SIZE).build(addr).await.unwrap();
    let module = make_module(ctx);
    debug!("rpc at {:?}", srv.local_addr().unwrap());
    srv.start(module).unwrap()
}

fn make_module(ctx: RpcContext) -> RpcModule<RpcContext> {
    let mut m = RpcModule::new(ctx);

    m.register_method("getmetrics", |_, c| Ok(c.summary.lock().clone())).unwrap();

    m.register_method("getstats", |_, c| {
        let nodes = c.nodes.lock();
        let tip = get_tip(&nodes);
        let mut contacted = 0; let mut relevant = 0;
        let mut zd = 0; let mut zb = 0; let mut fx = 0; let mut ot = 0;

        for (_, n) in nodes.iter() {
            if n.user_agent.is_none() { continue; }
            contacted += 1;
            let ua = n.user_agent.as_ref().map(|x| x.0.clone()).unwrap_or_default();
            let (t, _) = client_type(&ua);
            match t.as_str() { "zcashd" => zd += 1, "zebra" => zb += 1, "flux" => fx += 1, _ => ot += 1 }
            if is_good_node(n, tip) { relevant += 1; }
        }
        Ok(Stats { num_known_nodes: nodes.len(), num_contacted_nodes: contacted, num_relevant_zcash_nodes: relevant,
            num_zcashd_nodes: zd, num_zebra_nodes: zb, num_flux_nodes: fx, num_other_nodes: ot,
            tip_height_estimate: tip, crawler_runtime_secs: c.summary.lock().crawler_runtime.as_secs() })
    }).unwrap();

    m.register_method("getnodes", |p, c| {
        let show_flux: bool = p.parse().unwrap_or(false);
        let rt = c.summary.lock().crawler_runtime.as_secs();
        let nodes = c.nodes.lock();
        let tip = get_tip(&nodes);

        let mut stats = Stats { num_known_nodes: nodes.len(), num_contacted_nodes: 0, num_relevant_zcash_nodes: 0,
            num_zcashd_nodes: 0, num_zebra_nodes: 0, num_flux_nodes: 0, num_other_nodes: 0, tip_height_estimate: tip, crawler_runtime_secs: rt };
        let mut out = Vec::new();

        for (addr, n) in nodes.iter() {
            if n.user_agent.is_none() { continue; }
            stats.num_contacted_nodes += 1;
            let ua = n.user_agent.as_ref().map(|x| x.0.clone()).unwrap_or_default();
            let (t, flux) = client_type(&ua);
            let good = is_good_node(n, tip);

            match t.as_str() { "zcashd" => stats.num_zcashd_nodes += 1, "zebra" => stats.num_zebra_nodes += 1, "flux" => stats.num_flux_nodes += 1, _ => stats.num_other_nodes += 1 }
            if good { stats.num_relevant_zcash_nodes += 1; }

            if !show_flux && flux { continue; }
            if !show_flux && t != "zcashd" && t != "zebra" { continue; }

            out.push(NodeInfo {
                ip: addr.ip().to_string(), port: addr.port(),
                protocol_version: n.protocol_version.map(|v| v.0), user_agent: Some(ua),
                height: n.start_height, services: n.services,
                last_seen_secs: n.last_connected.map(|t| t.elapsed().as_secs()).unwrap_or(u64::MAX),
                is_relevant: good, is_flux: flux, client_type: t,
            });
        }
        out.sort_by(|a, b| b.is_relevant.cmp(&a.is_relevant).then(a.last_seen_secs.cmp(&b.last_seen_secs)));
        Ok(NodesResponse { stats, nodes: out })
    }).unwrap();

    m
}
