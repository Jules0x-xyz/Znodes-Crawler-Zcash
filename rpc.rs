use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use jsonrpsee::server::{RpcModule, ServerBuilder, ServerHandle};
use parking_lot::Mutex;
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::debug;
use ziggurat_core_crawler::summary::NetworkSummary;

use crate::network::KnownNode;

// Tolerancia de altura respecto al tip (configurable)
const MAX_HEIGHT_DELTA: i32 = 10000;

// Altura mínima para mainnet (bloque actual ~2.7M)
const MIN_MAINNET_HEIGHT: i32 = 2_500_000;

// Puertos estándar de Zcash
const ZCASH_MAINNET_PORT: u16 = 8233;
const ZCASH_TESTNET_PORT: u16 = 18233;

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

/// Estima el tip height usando el percentil 95 de alturas válidas
fn estimate_tip_height(nodes: &HashMap<SocketAddr, KnownNode>) -> i32 {
    let mut heights: Vec<i32> = nodes
        .values()
        .filter_map(|n| n.start_height)
        .filter(|h| *h > MIN_MAINNET_HEIGHT)
        .collect();
    
    if heights.is_empty() {
        return MIN_MAINNET_HEIGHT;
    }
    
    heights.sort();
    let idx = ((heights.len() as f64) * 0.95) as usize;
    heights.get(idx.min(heights.len() - 1)).copied().unwrap_or(MIN_MAINNET_HEIGHT)
}

/// Clasifica el tipo de cliente basado en user_agent
fn classify_client(user_agent: &str) -> (String, bool) {
    let ua_lower = user_agent.to_lowercase();
    
    // Detectar Flux (contiene "flux" en cualquier parte)
    let is_flux = ua_lower.contains("flux");
    
    if is_flux {
        return ("flux".to_string(), true);
    }
    
    // MagicBean 6.x también es Flux
    if ua_lower.contains("magicbean:6.") {
        return ("flux".to_string(), true);
    }
    
    // Zcashd (MagicBean sin flux)
    if ua_lower.contains("magicbean") {
        return ("zcashd".to_string(), false);
    }
    
    // Zebra
    if ua_lower.contains("zebra") {
        return ("zebra".to_string(), false);
    }
    
    ("other".to_string(), false)
}

/// Determina si un nodo es relevante para Zcash mainnet
fn is_relevant_zcash_node(
    node: &KnownNode,
    port: u16,
    tip_height: i32,
) -> bool {
    // 1. Debe haber respondido VERSION (tener user_agent)
    let user_agent = match &node.user_agent {
        Some(ua) => ua.0.clone(),
        None => return false,
    };
    
    let ua_lower = user_agent.to_lowercase();
    
    // 2. User agent debe empezar con /MagicBean o /Zebra
    let valid_client = ua_lower.starts_with("/magicbean") || ua_lower.starts_with("/zebra");
    if !valid_client {
        return false;
    }
    
    // 3. NO debe contener "flux"
    if ua_lower.contains("flux") {
        return false;
    }
    
    // 4. MagicBean 6.x es Flux, excluir
    if ua_lower.contains("magicbean:6.") {
        return false;
    }
    
    // 5. Altura debe estar dentro del rango permitido
    let height = node.start_height.unwrap_or(0);
    if height < MIN_MAINNET_HEIGHT {
        return false;
    }
    
    let height_diff = (height - tip_height).abs();
    if height_diff > MAX_HEIGHT_DELTA {
        return false;
    }
    
    // 6. Puerto preferiblemente estándar (pero no excluyente)
    // Los nodos en puertos estándar tienen prioridad pero no excluimos otros
    
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

    // Método original para compatibilidad
    module.register_method("getmetrics", |_, ctx| {
        Ok(ctx.summary.lock().clone())
    }).unwrap();

    // Nuevo método con estadísticas filtradas
    module.register_method("getstats", |_, ctx| {
        let nodes = ctx.nodes.lock();
        let summary = ctx.summary.lock();
        let tip_height = estimate_tip_height(&nodes);
        
        let mut contacted = 0;
        let mut relevant = 0;
        let mut zcashd = 0;
        let mut zebra = 0;
        let mut flux = 0;
        let mut other = 0;
        
        for (addr, node) in nodes.iter() {
            // Solo contar nodos que respondieron VERSION
            if node.user_agent.is_none() {
                continue;
            }
            
            contacted += 1;
            
            let ua = node.user_agent.as_ref().map(|v| v.0.clone()).unwrap_or_default();
            let (client_type, is_flux) = classify_client(&ua);
            
            match client_type.as_str() {
                "zcashd" => zcashd += 1,
                "zebra" => zebra += 1,
                "flux" => flux += 1,
                _ => other += 1,
            }
            
            if is_relevant_zcash_node(node, addr.port(), tip_height) {
                relevant += 1;
            }
        }
        
        Ok(NetworkStats {
            num_known_nodes: nodes.len(),
            num_contacted_nodes: contacted,
            num_relevant_zcash_nodes: relevant,
            num_zcashd_nodes: zcashd,
            num_zebra_nodes: zebra,
            num_flux_nodes: flux,
            num_other_nodes: other,
            tip_height_estimate: tip_height,
            crawler_runtime_secs: summary.crawler_runtime.as_secs(),
        })
    }).unwrap();

    // Método para obtener nodos con filtrado opcional
    module.register_method("getnodes", |params, ctx| {
        let include_flux: bool = params.parse().unwrap_or(false);
        
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
            crawler_runtime_secs: 0,
        };
        
        for (addr, node) in nodes.iter() {
            // Solo nodos que respondieron VERSION
            if node.user_agent.is_none() {
                continue;
            }
            
            stats.num_contacted_nodes += 1;
            
            let ua = node.user_agent.as_ref().map(|v| v.0.clone()).unwrap_or_default();
            let (client_type, is_flux) = classify_client(&ua);
            let is_relevant = is_relevant_zcash_node(node, addr.port(), tip_height);
            
            match client_type.as_str() {
                "zcashd" => stats.num_zcashd_nodes += 1,
                "zebra" => stats.num_zebra_nodes += 1,
                "flux" => stats.num_flux_nodes += 1,
                _ => stats.num_other_nodes += 1,
            }
            
            if is_relevant {
                stats.num_relevant_zcash_nodes += 1;
            }
            
            // Filtrar según parámetro
            if !include_flux && is_flux {
                continue;
            }
            
            // Por defecto solo mostrar nodos relevantes
            if !include_flux && !is_relevant {
                continue;
            }
            
            let last_seen_secs = node.last_connected
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
        
        // Ordenar: relevantes primero, luego por última conexión
        result.sort_by(|a, b| {
            b.is_relevant.cmp(&a.is_relevant)
                .then(a.last_seen_secs.cmp(&b.last_seen_secs))
        });
        
        Ok(NodesResponse { stats, nodes: result })
    }).unwrap();

    module
}
