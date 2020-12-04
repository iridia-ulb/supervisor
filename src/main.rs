use ipnet::Ipv4Net;
use std::sync::Arc;
use tokio::sync::RwLock;
use warp::Filter;

mod robot;
mod network;
mod webui;
mod experiment;
mod optitrack;
mod firmware;

type Robots = Arc<RwLock<Vec<robot::Robot>>>;
type Experiment = Arc<RwLock<experiment::Experiment>>;

#[tokio::main]
async fn main() {
    /* initialize the logger */
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("mns_supervisor=info")).init();
    /* create data structures for tracking the robots and state of the experiment */
    let robots = Robots::default();
    let experiment = Experiment::default();
    
    /* create a task for discovering robots connected to our network */
    let network = "192.168.1.0/24".parse::<Ipv4Net>().unwrap();
    let discovery_task = network::discover(network, robots.clone());
    
    /* create a task for coordinating with the webui */
    let experiment_clone = experiment.clone();
    let robots_filter = warp::any().map(move || robots.clone());
    let experiment_filter = warp::any().map(move || experiment_clone.clone());
    
    // TODO find a better solution for these hardcoded strings
    let index_route = warp::path::end().and(warp::fs::file(
        "/home/mallwright/Workspace/mns-supervisor/index.html",
    ));
    let static_route =
        warp::path("static").and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(robots_filter)
        .and(experiment_filter)
        .map(|ws: warp::ws::Ws, 
              robots : Robots,
              experiment : Experiment | {
            // This will call our function if the handshake succeeds.
            ws.on_upgrade(move |socket| webui::run(socket, robots, experiment))
        });

    let server_task = warp::serve(index_route.or(static_route).or(socket_route)).run(([127, 0, 0, 1], 3030));

    

    // TODO spawn tokio tasks at the end of this function so that it is clear what is running in parallel    
    let server_task_handle = tokio::task::spawn(server_task);
    let discovery_task_handle = tokio::task::spawn(discovery_task);

    
    // use try_join here? this will abort other tasks, when one task throws an error?
    let (server_task_res, discover_task_res) = 
        tokio::join!(server_task_handle, discovery_task_handle);

    if let Err(err) = server_task_res {
        eprintln!("Joining server task failed: {}", err);
    }

    if let Err(err) = discover_task_res {
        eprintln!("Joining discover task failed: {}", err);
    }
}