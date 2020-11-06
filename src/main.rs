use std::{sync::Arc, collections::HashMap};
use futures::{FutureExt, StreamExt};
use tokio::{
    sync::{ mpsc, RwLock },
};
use warp::{
    ws::{ Message, WebSocket },
    Filter
};
use serde::{
    Deserialize,
    Serialize
};

mod robots;

use robots::{
    drone::Drone,
    pipuck::PiPuck,
};

use ipnet::Ipv4Net;


// To consider, impl State for Drone, impl Action for Drone?
#[derive(Serialize, Deserialize, Debug)]
enum PiPuckAction {
    Shutdown,
}

// To consider, impl State for Drone, impl Action for Drone?


// serde lower case
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum GuiMessage {
    Update(String),
    // uuid, action
    Drone(uuid::Uuid, robots::drone::Action),
    // uuid, action
    PiPuck(uuid::Uuid, robots::pipuck::Action),
    //
    Emergency,
}

type Drones = Arc<RwLock<Vec<Drone>>>;
type PiPucks = Arc<RwLock<Vec<PiPuck>>>;

#[derive(Serialize, Debug)]
struct GuiCard {
    span: u8,
    title: String,
    content: String,
    actions: Vec<robots::drone::Action>
}

#[derive(Serialize, Debug)]
struct GuiContent {
    title: String,
    cards: HashMap<String, GuiCard>
}

#[tokio::main]
async fn main() {
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
    
    let drones = Drones::default();
    let pipucks = PiPucks::default();
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
        .map(|ws: warp::ws::Ws, drones : Drones, pipucks : PiPucks| {
            // This will call our function if the handshake succeeds.
            ws.on_upgrade(move |socket| connected(socket, drones, pipucks))
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

async fn connected(ws: WebSocket, drones: Drones, pipucks: PiPucks) {
    // Use a counter to assign a new unique ID for this user.

    eprintln!("websocket connected!");

    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("websocket send error: {}", e);
        }
    }));

    // this loop is basically our gui updating thread
    while let Some(data) = user_ws_rx.next().await {
        let message = match data {
            Ok(message) => message,
            Err(error) => {
                eprintln!("websocket receive error {}", error);
                break;
            }
        };

        if let Ok(message) = message.to_str() {
            if let Ok(action) = serde_json::from_str::<GuiMessage>(message) {
                match action {
                    GuiMessage::Update(view) => {
                        //eprintln!("selected view = {}", view);
            
                        let mut content = GuiContent {
                            title: String::from("Connections"),
                            cards: HashMap::new(),
                        };
            
                        for drone in drones.read().await.iter() {
                            let card = GuiCard {
                                span: 4,
                                title: String::from("Drone"),
                                content: format!("{:?}", drone),
                                // the actions depend on the state of the drone
                                // the action part of the message must contain
                                // the uuid, action name, and optionally arguments
                                actions: drone.actions(),
                            };
                            content.cards.insert(drone.uuid.to_string(), card);
                        }

                        for pipuck in pipucks.read().await.iter() {
                            let card = GuiCard {
                                span: 4,
                                title: String::from("PiPuck"),
                                content: format!("{:?}", pipuck),
                                // the actions depend on the state of the drone
                                // the action part of the message must contain
                                // the uuid, action name, and optionally arguments
                                actions: vec![], //pipuck.actions(),
                            };
                            content.cards.insert(pipuck.uuid.to_string(), card);
                        }
            
                        if let Ok(reply) = serde_json::to_string(&content) {
                            //eprintln!("{} -> {}", view, reply);
                            if let Err(_) = tx.send(Ok(Message::text(reply))) {
                                // do nothing?
                            }
                        }
                    },
                    GuiMessage::Drone(uuid, action) => {
                        if let Some(drone) = 
                            drones.write().await
                                  .iter_mut()
                                  .find(|drone| drone.uuid == uuid) {
                            /* check if the action is still valid given the drones current state */
                            if drone.actions().contains(&action) {
                                let result = match action {
                                    robots::drone::Action::UpCorePowerOn => {
                                        // TODO, is it possible to not block the collection here?
                                        // What if just the xbee was locked during the await?
                                        drone.set_power(Some(true), None).await
                                    },
                                    _ => todo!()
                                };
                                eprintln!("{:?}", result);
                            }
                        }
                    },
                    _ => {
                        // TODO handle drone and pipuck messages
                    }
                }
            }
            else {
                eprintln!("[warning] could not deserialize message");
            }
        }
    }

    eprintln!("websocket disconnected!");
}
