use std::{io, net::{Ipv4Addr, SocketAddr}, path::{Path, PathBuf}, str::FromStr};
use anyhow::Context;
use base64::Config;
use ipnet::Ipv4Net;
use journal::Robot;
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
#[structopt(name = "supervisor", about = "A supervisor for experiments with swarms of robots")]
struct Options {
    #[structopt(short = "c", long = "configuration")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    /* initialize the logger */
    let environment = env_logger::Env::default().default_filter_or("supervisor=info");
    env_logger::Builder::from_env(environment).format_timestamp_millis().init();
    /* parse the configuration file */
    let options = Options::from_args();
    let config = match read_config(&options.config) {
        Ok(config) => config,
        Err(err) => {
            log::error!("Configuration error: {}", err);
            return;
        }
    };
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
    let network_task = network::new(config.robot_network, &arena_requests_tx);
    /* create message router task */
    let message_router_addr : SocketAddr = (Ipv4Addr::UNSPECIFIED, 4950).into();
    let router_task = router::new(message_router_addr, journal_requests_tx.clone());
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
        .and(static_dir::static_dir!("static"));
    //    .and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let server_addr : SocketAddr = (Ipv4Addr::LOCALHOST, 3030).into();
    let webui_task = warp::serve(socket_route.or(static_route)).run(server_addr);
    /* pin the futures so that they can be polled via &mut */
    tokio::pin!(arena_task);
    tokio::pin!(journal_task);
    tokio::pin!(network_task);
    tokio::pin!(webui_task);
    tokio::pin!(sigint_task);

    let mut router_task = tokio::spawn(router_task);
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

/*
<?xml version="1.0" ?>
<configuration>
    <supervisor>
        <router socket="localhost:1234" />
        <webui socket="localhost:8000" />
    </supervisor>
    <robots network="192.168.1.0/24">
        <drone  id="drone1" 
                xbee_addr="FFFFFFFFFFFF"
                upcore_addr="FFFFFFFFFFFF"
                optitrack_id="1" />
        <pipuck id="pipuck1" 
                rpi_addr="FFFFFFFFFFFF"
                optitrack_id="2"
                apriltag_id="20" />
    </robots>
</configuration>

 */

enum RobotTableEntry {
    Drone {
        id: String,
        xbee_macaddr: [u8; 6],
        upcore_macaddr: [u8; 6],
        optitrack_id: u8,
    },
    PiPuck {
        id: String,
        rpi_macaddr: [u8; 6],
        optitrack_id: u8,
        apriltag_id: u8,
    }
}

struct Configuration {
    router_socket: Option<SocketAddr>,
    webui_socket: Option<SocketAddr>,
    robot_network: Ipv4Net,
    robot_table: Vec<RobotTableEntry>,
}

fn read_config(config: &Path) -> anyhow::Result<Configuration> {
    let config = std::fs::read_to_string(config)
        .context("Could not read configuration file")?;
    let xml = roxmltree::Document::parse(&config)
        .context("Could not parse configuration file")?;
    let supervisor_config = xml
        .descendants()
        .find(|a| a.tag_name().name() == "supervisor")
        .ok_or("Could not find tag \"supervisor\"");


    Ok(Configuration {})
}