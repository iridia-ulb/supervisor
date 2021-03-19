use std::net::{Ipv4Addr, SocketAddr};

use ipnet::Ipv4Net;
use tokio::sync::mpsc;
use warp::Filter;
use structopt::StructOpt;

mod arena;
mod robot;
mod network;
mod webui;
mod optitrack;
mod software;
mod journal;
mod router;

#[derive(Debug, StructOpt)]
#[structopt(name = "mns-supervisor", about = "A supervisor for the MNS experiments")]
struct Options {
    #[structopt(long)]
    network: Ipv4Net,
}

#[tokio::main]
async fn main() {
    let options = Options::from_args();
    /* initialize the logger */
    let environment = env_logger::Env::default().default_filter_or("mns_supervisor=info");
    env_logger::Builder::from_env(environment).init();
    /* create a task for tracking the robots and state of the experiment */
    let (arena_requests_tx, arena_requests_rx) = mpsc::unbounded_channel();
    let (journal_requests_tx, journal_requests_rx) = mpsc::unbounded_channel();
    /* listen for the ctrl-c shutdown signal */
    let sigint_task = tokio::signal::ctrl_c();
    /* create journal task */
    let journal_task = journal::new(journal_requests_rx);
    /* create arena task */
    let arena_task = arena::new(arena_requests_rx, &journal_requests_tx);
    /* create network task */
    let network_task = network::new(options.network, &arena_requests_tx);
    /* create message router task */
    let message_router_addr : SocketAddr = (Ipv4Addr::UNSPECIFIED, 4950).into();
    let router_task = router::new(message_router_addr, &journal_requests_tx);
    /* create webui task */
    /* clone arena requests tx for moving into the closure */
    let arena_requests_tx = arena_requests_tx.clone();
    let arena_channel = warp::any().map(move || arena_requests_tx.clone());
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(arena_channel)
        .map(|websocket: warp::ws::Ws, arena_requests_tx| {
            websocket.on_upgrade(move |socket| webui::run(socket, arena_requests_tx))
        });
    let static_route = warp::get()
    //    .and(static_dir::static_dir!("static"));
        .and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let server_addr : SocketAddr = (Ipv4Addr::UNSPECIFIED, 3030).into();
    let webui_task = warp::serve(socket_route.or(static_route)).run(server_addr);
    /* pin the futures so that they can be polled via &mut */
    tokio::pin!(arena_task);
    tokio::pin!(journal_task);
    tokio::pin!(network_task);
    tokio::pin!(router_task);
    tokio::pin!(webui_task);
    tokio::pin!(sigint_task);

    /* no point in implementing automatic browser opening */
    /* https://bugzilla.mozilla.org/show_bug.cgi?id=1512438 */
    let server_addr = format!("http://{}/", server_addr);
    if let Err(_) = webbrowser::open(&server_addr) {
        log::warn!("Could not start browser");
        log::info!("Please open this URL manually: {}", server_addr);
    };
    
    tokio::select! {
        _ = &mut arena_task => {},
        _ = &mut journal_task => {},
        _ = &mut network_task => {},
        _ = &mut router_task => {},
        _ = &mut webui_task => {},
        _ = &mut sigint_task => {
            /* TODO: is it safe to do this? should messages be broadcast to robots */
            /* what happens if ARGoS is running on the robots, does breaking the
            connection to fernbedienung kill ARGoS? How does the Pixhawk respond */
            log::info!("Shutting down");
        }
    }
}