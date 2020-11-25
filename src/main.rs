use futures::StreamExt;
use ipnet::Ipv4Net;
use robots::{
    drone::Drone,
    pipuck::PiPuck,
};
use std::sync::Arc;
use tokio::{
    sync::{ mpsc, RwLock },
};
use warp::Filter;

mod robots;
mod webui;
mod experiment;
mod optitrack;

type Robots<T> = Arc<RwLock<Vec<T>>>;

#[tokio::main]
async fn main() {
    /* init the backend logger */
    // TODO only show messages from the current module mns_supervisor
    // TODO fix issue with PiPuck actions: why is action always Identify
    // (it looks like this problem is JS related to how closures bind to variables)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("mns_supervisor=info")).init();

    let (addr_tx, addr_rx) = mpsc::unbounded_channel();
    let (assoc_tx, mut assoc_rx) = mpsc::unbounded_channel();

    /* pass all addresses from the robot network down the
       channel and try associated them with robots */
    let addrs = "192.168.1.0/24"
        .parse::<Ipv4Net>()
        .unwrap()
        .hosts();
    for addr in addrs {
        addr_tx.send(addr).unwrap();
    }
    
    /* spawn the task for associating addresses with robots */
    let task_discover = 
        tokio::task::spawn(robots::discover(addr_rx, assoc_tx));
    
    let drones = Robots::default();
    let pipucks = Robots::default();
    let drones_clone = drones.clone();
    let pipuck_clone = pipucks.clone();
    let drones_filter = warp::any().map(move || drones_clone.clone());
    let pipuck_filter = warp::any().map(move || pipuck_clone.clone());


    let task_temp = tokio::task::spawn(async move {
        while let Some(device) = assoc_rx.next().await {
            match device {
                robots::Device::Ssh(mut device) => {
                    if let Ok(hostname) = device.hostname().await {
                        match &hostname[..] {
                            "raspberrypi0-wifi" => {
                                let pipuck = PiPuck::new(device);
                                pipucks.write().await.push(pipuck);
                            },
                            _ => {
                                eprintln!("[warning] {} accepted the SSH \
                                          connection with root login, but the \
                                          hostname ({}) was not recognised",
                                          hostname, device.addr);
                                // place back in the pool with 5 second delay
                            }
                        }
                    }
                    else {
                        // getting hostname failed
                        // place back in the pool with 1 second delay
                    }
                },
                robots::Device::Xbee(device) => {
                    let mut drone = Drone::new(device);
                    if let Ok(_) = drone.init().await {
                        drones.write().await.push(drone);
                    }
                    else {
                        // place address back in pool
                    }
                }
            }
        }
    });
    
    let index_route = warp::path::end().and(warp::fs::file(
        "/home/mallwright/Workspace/mns-supervisor/index.html",
    ));
    let static_route =
        warp::path("static").and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(drones_filter)
        .and(pipuck_filter)
        .map(|ws: warp::ws::Ws, drones : Robots<Drone>, pipucks : Robots<PiPuck>| {
            // This will call our function if the handshake succeeds.
            ws.on_upgrade(move |socket| webui::run(socket, drones, pipucks))
        });

    // does the 'async move' all referenced variables into the lambda?
    let task_server = tokio::task::spawn(async move { 
        warp::serve(index_route.or(static_route).or(socket_route))
        .run(([127, 0, 0, 1], 3030)).await
    });

    
    // use try_join here? this will abort other tasks, when one task throws an error?
    let (server_task_res, discover_task_res, temp_task_res) = 
        tokio::join!(task_server, task_discover, task_temp);

    if let Err(err) = temp_task_res {
        eprintln!("Joining temp task failed: {}", err);
    }

    if let Err(err) = server_task_res {
        eprintln!("Joining server task failed: {}", err);
    }

    if let Err(err) = discover_task_res {
        eprintln!("Joining discover task failed: {}", err);
    }
}