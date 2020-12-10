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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("mns_supervisor=info")).init();
    /* create data structures for tracking the robots and state of the experiment */
    let robots = Robots::default();
    let experiment = Arc::new(RwLock::new(experiment::Experiment::with_robots(robots.clone())));
    
    /* create a task for discovering robots connected to our network */
    let network = "192.168.1.0/24".parse::<Ipv4Net>().unwrap();
    let discovery_task = network::discover(network, robots.clone());
    
    /* create a task for coordinating with the webui */
    let robots_filter = warp::any().map(move || robots.clone());
    let experiment_filter = warp::any().map(move || experiment.clone());
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(robots_filter)
        .and(experiment_filter)
        .map(|ws: warp::ws::Ws, 
                robots : Robots,
                experiment : Experiment | {
            /* start an instance of the webui if handshake is successful */
            ws.on_upgrade(move |socket| webui::run(socket, robots, experiment))
        });
    let static_route = warp::get()
    //    .and(static_dir::static_dir!("static"));
        .and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));

    let server_task = warp::serve(socket_route.or(static_route)).run(([127, 0, 0, 1], 3030));
    


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