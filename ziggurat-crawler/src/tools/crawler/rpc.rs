use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use jsonrpsee::server::{RpcModule, ServerBuilder, ServerHandle};
use parking_lot::Mutex;
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::debug;
use ziggurat_core_crawler::summary::NetworkSummary;

use crate::network::KnownNode;

pub const MAX_RESPONSE_SIZE: u32 = 200_000_000;

#[derive(Clone, Serialize)]
pub struct NodeInfo {
    pub ip: String,
    pub port: u16,
    pub protocol_version: Option<u32>,
    pub user_agent: Option<String>,
    pub height: Option<i32>,
    pub services: Option<u64>,
    pub last_seen_secs: u64,
    pub is_relevant: bool,
    pub is_flux: bool,
    pub client_type: String,
}

#[derive(Clone, Serialize)]
pub struct NetworkStats {
    pub num_known_nodes: usize,
    pub num_contacted_nodes: usize,
    pub num_relevant_zcash_nodes: usize,
    pub num_zcashd_nodes: usize,
    pub num_zebra_nodes: usize,
    pub num_flux_nodes: usize,
    pub num_other_nodes: usize,
    pub tip_height_estimate: i32,
    pub crawler_runtime_secs: u64,
}

#[derive(Clone, Serialize)]
pub struct NodesResponse {
    pub stats: NetworkStats,
    pub nodes: Vec<NodeInfo>,
}

#[derive(Clone, Serialize)]
pub struct NodeGeo {
    pub ip: String,
    pub client_type: String,
    pub height: Option<i32>,
}

#[derive(Clone, Serialize)]
pub struct DiagnosticInfo {
    pub total_known: usize,
    pub total_contacted: usize,
    pub filtered_by_no_ua: usize,
    pub filtered_by_flux: usize,
    pub filtered_by_height: usize,
    pub filtered_by_sync: usize,
    pub passed_filters: usize,
    pub zcashd_nodes: usize,
    pub zebra_nodes: usize,
}

pub struct RpcContext {
    summary: Arc<Mutex<NetworkSummary>>,
    nodes: Arc<Mutex<HashMap<SocketAddr, KnownNode>>>,
}

impl RpcContext {
    pub fn new(
        summary: Arc<Mutex<NetworkSummary>>,
        nodes: Arc<Mutex<HashMap<SocketAddr, KnownNode>>>,
    ) -> RpcContext {
        RpcContext { summary, nodes }
    }
}

fn estimate_tip_height(nodes: &HashMap<SocketAddr, KnownNode>) -> i32 {
    let mut heights: Vec<i32> = nodes
        .values()
        .filter_map(|n| n.start_height)
        .filter(|h| *h > 2_500_000)
        .collect();

    if heights.is_empty() {
        return 3_150_000;
    }

    heights.sort();
    let idx = ((heights.len() as f64) * 0.95) as usize;
    heights.get(idx.min(heights.len() - 1)).copied().unwrap_or(3_150_000)
}

fn classify_client(user_agent: &str) -> (String, bool) {
    let ua_lower = user_agent.to_lowercase();

    // Solo es Flux si explícitamente dice "flux" en el user agent
    if ua_lower.contains("flux") {
        return ("flux".to_string(), true);
    }

    // MagicBean = zcashd (todas las versiones)
    if ua_lower.contains("magicbean") {
        return ("zcashd".to_string(), false);
    }

    if ua_lower.contains("zebra") {
        return ("zebra".to_string(), false);
    }

    ("other".to_string(), false)
}

fn is_relevant_zcash_node(node: &KnownNode, tip_height: i32) -> bool {
    let user_agent = match &node.user_agent {
        Some(ua) => ua.0.clone(),
        None => return false,
    };

    let ua_lower = user_agent.to_lowercase();
    let (client_type, is_flux) = classify_client(&user_agent);
    
    if is_flux || client_type == "other" {
        return false;
    }

    let height = node.start_height.unwrap_or(0);
    if height < 2_500_000 {
        return false;
    }

    // Nodos zcashd pueden estar más atrasados (syncing), zebra más cerca del tip
    let height_diff = (height - tip_height).abs();
    if client_type == "zebra" && height_diff > 20000 {
        return false;
    }
    // zcashd puede estar sincronizando, ser más permisivo
    if client_type == "zcashd" && height_diff > 100000 {
        return false;
    }

    true
}

pub async fn initialize_rpc_server(rpc_addr: SocketAddr, rpc_context: RpcContext) -> ServerHandle {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let middleware = tower::ServiceBuilder::new().layer(cors);

    let server = ServerBuilder::default()
        .set_middleware(middleware)
        .max_response_body_size(MAX_RESPONSE_SIZE)
        .build(rpc_addr)
        .await
        .unwrap();

    let module = create_rpc_module(rpc_context);
    debug!("Starting RPC server at {:?}", server.local_addr().unwrap());
    server.start(module).unwrap()
}

fn create_rpc_module(rpc_context: RpcContext) -> RpcModule<RpcContext> {
    let mut module = RpcModule::new(rpc_context);

    module
        .register_method("getmetrics", |_, ctx| Ok(ctx.summary.lock().clone()))
        .unwrap();

    module
        .register_method("getnodes", |params, ctx| {
            let include_flux: bool = params.parse().unwrap_or(false);
            let runtime_secs = ctx.summary.lock().crawler_runtime.as_secs();

            let nodes = ctx.nodes.lock();
            let tip_height = estimate_tip_height(&nodes);

            let mut result: Vec<NodeInfo> = Vec::new();
            let mut stats = NetworkStats {
                num_known_nodes: nodes.len(),
                num_contacted_nodes: 0,
                num_relevant_zcash_nodes: 0,
                num_zcashd_nodes: 0,
                num_zebra_nodes: 0,
                num_flux_nodes: 0,
                num_other_nodes: 0,
                tip_height_estimate: tip_height,
                crawler_runtime_secs: runtime_secs,
            };

            for (addr, node) in nodes.iter() {
                if node.user_agent.is_none() {
                    continue;
                }

                stats.num_contacted_nodes += 1;

                let ua = node.user_agent.as_ref().map(|v| v.0.clone()).unwrap_or_default();
                let (client_type, is_flux) = classify_client(&ua);
                let is_relevant = is_relevant_zcash_node(node, tip_height);

                // Solo contar si no es flux
                if !is_flux {
                    match client_type.as_str() {
                        "zcashd" => if is_relevant { stats.num_zcashd_nodes += 1; },
                        "zebra" => if is_relevant { stats.num_zebra_nodes += 1; },
                        _ => {}
                    }
                } else {
                    stats.num_flux_nodes += 1;
                }

                if is_relevant {
                    stats.num_relevant_zcash_nodes += 1;
                }

                // Solo mostrar nodos relevantes (sincronizados) y no-flux
                if !is_relevant || is_flux {
                    continue;
                }

                let is_zcash_node = client_type == "zcashd" || client_type == "zebra";
                if !is_zcash_node {
                    continue;
                }

                let last_seen_secs = node
                    .last_connected
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(u64::MAX);

                result.push(NodeInfo {
                    ip: addr.ip().to_string(),
                    port: addr.port(),
                    protocol_version: node.protocol_version.map(|v| v.0),
                    user_agent: Some(ua),
                    height: node.start_height,
                    services: node.services,
                    last_seen_secs,
                    is_relevant,
                    is_flux,
                    client_type,
                });
            }

            result.sort_by(|a, b| {
                b.is_relevant
                    .cmp(&a.is_relevant)
                    .then(a.last_seen_secs.cmp(&b.last_seen_secs))
            });

            Ok(NodesResponse { stats, nodes: result })
        })
        .unwrap();

    module
        .register_method("getgeonodes", |_, ctx| {
            let nodes = ctx.nodes.lock();
            let tip_height = estimate_tip_height(&nodes);
            let mut result = Vec::new();

            for (addr, node) in nodes.iter() {
                if node.user_agent.is_none() {
                    continue;
                }

                let ua = node.user_agent.as_ref().map(|v| v.0.clone()).unwrap_or_default();
                let (client_type, is_flux) = classify_client(&ua);
                let is_relevant = is_relevant_zcash_node(node, tip_height);

                if is_flux || !is_relevant {
                    continue;
                }

                if client_type != "zcashd" && client_type != "zebra" {
                    continue;
                }

                result.push(NodeGeo {
                    ip: addr.ip().to_string(),
                    client_type,
                    height: node.start_height,
                });
            }

            Ok(result)
        })
        .unwrap();

    module
        .register_method("getdiagnostics", |_, ctx| {
            let nodes = ctx.nodes.lock();
            let tip_height = estimate_tip_height(&nodes);

            let mut no_ua = 0;
            let mut flux_count = 0;
            let mut height_filtered = 0;
            let mut sync_filtered = 0;
            let mut passed = 0;
            let mut zcashd = 0;
            let mut zebra = 0;

            for node in nodes.values() {
                if node.user_agent.is_none() {
                    no_ua += 1;
                    continue;
                }

                let ua = node.user_agent.as_ref().map(|v| v.0.clone()).unwrap_or_default();
                let (client_type, is_flux) = classify_client(&ua);

                if is_flux {
                    flux_count += 1;
                    continue;
                }

                if client_type == "other" {
                    continue;
                }

                let height = node.start_height.unwrap_or(0);
                if height < 2_500_000 {
                    height_filtered += 1;
                    continue;
                }

                let height_diff = (height - tip_height).abs();
                let sync_ok = if client_type == "zebra" {
                    height_diff <= 20000
                } else {
                    height_diff <= 100000
                };

                if !sync_ok {
                    sync_filtered += 1;
                    continue;
                }

                passed += 1;
                if client_type == "zcashd" {
                    zcashd += 1;
                } else if client_type == "zebra" {
                    zebra += 1;
                }
            }

            Ok(DiagnosticInfo {
                total_known: nodes.len(),
                total_contacted: nodes.values().filter(|n| n.user_agent.is_some()).count(),
                filtered_by_no_ua: no_ua,
                filtered_by_flux: flux_count,
                filtered_by_height: height_filtered,
                filtered_by_sync: sync_filtered,
                passed_filters: passed,
                zcashd_nodes: zcashd,
                zebra_nodes: zebra,
            })
        })
        .unwrap();

    module
}
