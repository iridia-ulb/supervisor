use std::net::SocketAddr;

use ipnet::Ipv4Net;
use tokio::sync::mpsc;
use warp::Filter;

mod arena;
mod robot;
mod network;
mod webui;
mod optitrack;
mod software;
mod journal;
mod router;

/// TODO:
/// 1. Clean up this code so that it compiles again [DONE]
/// 1a. Quick investigation into what a robot enum (static dispatch) would look like [DONE]
/// 1b. Use static_dir to embedded static resources inside of the app [DONE]
/// 1c. Start experiment triggers upload to all robots? [DONE]
/// 1d. Add basic control software checks, is there one .argos file, are all referenced files included?
/// 1e. Start ARGoS with the controller and shutdown at end of experiment 
/// 2. Add the ping functionality to remove robots if they don't reply
///    a. What if SSH drops from drone, but Xbee is still up? (move back to the standby state?)
///    b. What if Xbee drops, but SSH is still up? (these are difficult problems to solve)
/// 3. Investigate the SSH shell drop outs (and add fallback code to tolerate this)
/// 4. Add code to catch SIGINT

#[tokio::main]
async fn main() {
    /* initialize the logger */
    let environment = env_logger::Env::default().default_filter_or("mns_supervisor=info");
    env_logger::Builder::from_env(environment).init();
    
    /* create a task for tracking the robots and state of the experiment */
    let (arena_requests_tx, arena_requests_rx) = mpsc::unbounded_channel();
    let (network_addr_tx, network_addr_rx) = mpsc::unbounded_channel();
    let (journal_requests_tx, journal_requests_rx) = mpsc::unbounded_channel();
    
    /* add all network addresses from the 192.168.1.0/24 subnet */
    for network_addr in "192.168.1.0/24".parse::<Ipv4Net>().unwrap().hosts() {
        network_addr_tx.send(network_addr).unwrap();
    }
    let message_router_addr : SocketAddr = ([127, 0, 0, 1], 4950).into();

    /* create journal task */
    let journal_task = journal::new(journal_requests_rx);

    /* create arena task */
    let arena_task = arena::new(message_router_addr, arena_requests_rx, &network_addr_tx, &journal_requests_tx);

    /* create network task */
    let network_task = network::new(network_addr_rx, &arena_requests_tx);
    
    /* create message router task */
    let router_socket_addr : SocketAddr = ([127, 0, 0, 1], 4950).into();
    let router_task = router::new(router_socket_addr, &journal_requests_tx);

    /* create webui task */
    /* clone arena requests tx for moving into the closure */
    let arena_requests_tx = arena_requests_tx.clone();
    let arena_filter = warp::any().map(move || arena_requests_tx.clone());
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(arena_filter)
        .map(|websocket: warp::ws::Ws, arena_requests_tx| {
            websocket.on_upgrade(move |socket| webui::run(socket, arena_requests_tx))
        });
    let static_route = warp::get()
    //    .and(static_dir::static_dir!("static"));
        .and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let server_addr : SocketAddr = ([127, 0, 0, 1], 3030).into();
    let webui_task = warp::serve(socket_route.or(static_route)).run(server_addr);

    /* run tasks to completion on this thread */
    let results = tokio::join!(arena_task, journal_task, network_task, router_task, webui_task);
    log::info!("shutdown ({:?})", results);
}