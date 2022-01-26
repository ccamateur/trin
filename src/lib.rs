use std::collections::HashMap;
use std::sync::Arc;

use log::debug;
use tokio::sync::{mpsc, RwLock};

use trin_core::jsonrpc::handlers::JsonRpcHandler;
use trin_core::jsonrpc::types::PortalJsonRpcRequest;
use trin_core::portalnet::events::PortalnetEvents;
use trin_core::utp::stream::UtpListener;
use trin_core::{
    cli::{TrinConfig, HISTORY_NETWORK, STATE_NETWORK},
    jsonrpc::service::{launch_jsonrpc_server, JsonRpcExiter},
    portalnet::{
        discovery::Discovery,
        storage::{DistanceFunction, PortalStorage, PortalStorageConfig},
        types::messages::PortalnetConfig,
    },
};
use trin_history::initialize_history_network;
use trin_state::initialize_state_network;

pub async fn run_trin(
    trin_config: TrinConfig,
    infura_project_id: String,
) -> Result<Arc<JsonRpcExiter>, Box<dyn std::error::Error>> {
    trin_config.display_config();

    let bootnode_enrs = trin_config
        .bootnodes
        .iter()
        .map(|nodestr| nodestr.parse().unwrap())
        .collect();

    let portalnet_config = PortalnetConfig {
        external_addr: trin_config.external_addr,
        private_key: trin_config.private_key.clone(),
        listen_port: trin_config.discovery_port,
        internal_ip: trin_config.internal_ip,
        bootnode_enrs,
        ..Default::default()
    };

    // Initialize base discovery protocol
    let mut discovery = Discovery::new(portalnet_config.clone()).unwrap();
    discovery.start().await.unwrap();
    let discovery = Arc::new(discovery);
    // Search for discv5 peers (bucket refresh lookup)
    tokio::spawn(Arc::clone(&discovery).bucket_refresh_lookup());

    // Initialize UTP listener
    let utp_listener = Arc::new(RwLock::new(UtpListener {
        discovery: Arc::clone(&discovery),
        utp_connections: HashMap::new(),
        listening: HashMap::new(),
    }));

    // Initialize DB config
    let node_id = discovery.local_enr().node_id();
    let rocks_db = PortalStorage::setup_rocksdb(node_id)?;
    let (sql_handler_tx, sql_handler_rx) = mpsc::channel(32);
    let sql_handler = PortalStorage::setup_sql(node_id, sql_handler_rx)?;
    let storage_config = PortalStorageConfig {
        storage_capacity_kb: (trin_config.kb / 4) as u64,
        node_id,
        distance_function: DistanceFunction::Xor,
        db: Arc::new(rocks_db),
        sql_handler_tx,
    };

    tokio::spawn(sql_handler.process_sql_requests());

    debug!("Selected networks to spawn: {:?}", trin_config.networks);
    // Initialize state sub-network service and event handlers, if selected
    let (state_handler, state_network_task, state_event_tx, state_jsonrpc_tx) =
        if trin_config.networks.iter().any(|val| val == STATE_NETWORK) {
            initialize_state_network(
                &discovery,
                &utp_listener,
                portalnet_config.clone(),
                storage_config.clone(),
            )
            .await
        } else {
            (None, None, None, None)
        };

    // Initialize chain history sub-network service and event handlers, if selected
    let (history_handler, history_network_task, history_event_tx, history_jsonrpc_tx) =
        if trin_config
            .networks
            .iter()
            .any(|val| val == HISTORY_NETWORK)
        {
            initialize_history_network(
                &discovery,
                &utp_listener,
                portalnet_config.clone(),
                storage_config.clone(),
            )
            .await
        } else {
            (None, None, None, None)
        };

    // Initialize json-rpc server
    let (portal_jsonrpc_tx, portal_jsonrpc_rx) = mpsc::unbounded_channel::<PortalJsonRpcRequest>();
    let jsonrpc_trin_config = trin_config.clone();
    let (live_server_tx, mut live_server_rx) = tokio::sync::mpsc::channel::<bool>(1);
    let json_exiter = Arc::new(JsonRpcExiter::new());
    {
        let json_exiter_clone = Arc::clone(&json_exiter);
        tokio::task::spawn_blocking(|| {
            launch_jsonrpc_server(
                jsonrpc_trin_config,
                infura_project_id,
                portal_jsonrpc_tx,
                live_server_tx,
                json_exiter_clone,
            );
        });
    }

    // Spawn main JsonRpc Handler
    let jsonrpc_discovery = Arc::clone(&discovery);
    let rpc_handler = JsonRpcHandler {
        discovery: jsonrpc_discovery,
        portal_jsonrpc_rx,
        state_jsonrpc_tx,
        history_jsonrpc_tx,
    };

    tokio::spawn(rpc_handler.process_jsonrpc_requests());

    if let Some(handler) = state_handler {
        tokio::spawn(handler.handle_client_queries());
    }
    if let Some(handler) = history_handler {
        tokio::spawn(handler.handle_client_queries());
    }

    let portal_events_discovery = Arc::clone(&discovery);

    tokio::spawn(async move {
        let events = PortalnetEvents::new(
            portal_events_discovery,
            utp_listener,
            history_event_tx,
            state_event_tx,
        )
        .await;
        events.process_discv5_requests().await;
    });

    if let Some(network) = history_network_task {
        tokio::spawn(async { network.await });
    }
    if let Some(network) = state_network_task {
        tokio::spawn(async { network.await });
    }

    let _ = live_server_rx.recv().await;
    live_server_rx.close();

    Ok(json_exiter)
}
