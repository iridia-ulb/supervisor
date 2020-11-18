use warp::ws;

use std::collections::HashMap;

use serde::{
    Deserialize,
    Serialize
};

use futures::{FutureExt, StreamExt};

use tokio::{
    sync::{ mpsc },
};

use super::{
    robots::{
        drone,
        pipuck,
    },
    Robots
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Bad request")]
    BadRequest,

    #[error(transparent)]
    JsonError(#[from] serde_json::Error),

    #[error("Could not reply to client")]
    ReplyError,
}    

pub type Result<T> = std::result::Result<T, Error>;


// serde lower case
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum Request {
    Update(String),
    // uuid, action
    Drone(uuid::Uuid, drone::Action),
    // uuid, action
    PiPuck(uuid::Uuid, pipuck::Action),
    //
    Emergency,
}

#[derive(Serialize, Debug)]
struct Card {
    span: u8,
    title: String,
    content: String,
    actions: Vec<drone::Action>
}

// TODO, Reply will probably need to be wrapped in a enum soon Reply::Update, Reply::XXX
#[derive(Serialize, Debug)]
struct Reply {
    title: String,
    cards: HashMap<String, Card>
}

pub async fn run(ws: ws::WebSocket, drones: Robots<drone::Drone>, pipucks: Robots<pipuck::PiPuck>) {
    // Use a counter to assign a new unique ID for this user.

    eprintln!("websocket connected!");

    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(err) = result {
            eprintln!("websocket send error: {}", err);
        }
    }));

    // this loop is basically our gui updating thread
    while let Some(data) = user_ws_rx.next().await {
        let request : ws::Message = match data {
            Ok(request) => request,
            Err(error) => {
                eprintln!("websocket receive error {}", error);
                break;
            }
        };

        if let Ok(request) = request.to_str() {
            if let Ok(action) = serde_json::from_str::<Request>(request) {
                match action {
                    Request::Update(view) => {
                        let reply = match &view[..] {
                            "connections" => {
                                let cards = render_connections(&drones, &pipucks).await;
                                let title = String::from("Connections");
                                Ok(Reply { title, cards})
                            }
                            _ => Err(Error::BadRequest),
                        };
                        let result = reply
                            .and_then(|inner| {
                                serde_json::to_string(&inner).map_err(|err| Error::JsonError(err))
                            }).and_then(|inner| {
                                let message = Ok(ws::Message::text(inner));
                                tx.send(message).map_err(|_| Error::ReplyError)
                            });
                        if let Err(err) = result {
                            eprintln!("TODO: Consider handling {}", err);
                        }
                    },
                    Request::Drone(uuid, action) => {
                        if let Some(drone) = 
                            drones.write().await
                                  .iter_mut()
                                  .find(|drone| drone.uuid == uuid) {
                            /* check if the action is still valid given the drones current state */
                            if drone.actions().contains(&action) {
                                let result = match action {
                                    drone::Action::UpCorePowerOn => {
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

async fn render_connections(drones: &Robots<drone::Drone>, pipucks: &Robots<pipuck::PiPuck>) -> HashMap<String, Card> {
    let mut cards = HashMap::new();
    for drone in drones.read().await.iter() {
        let card = Card {
            span: 4,
            title: String::from("Drone"),
            content: format!("{:?}", drone),
            // the actions depend on the state of the drone
            // the action part of the message must contain
            // the uuid, action name, and optionally arguments
            actions: drone.actions(),
        };
        cards.insert(drone.uuid.to_string(), card);
    }
    for pipuck in pipucks.read().await.iter() {
        let card = Card {
            span: 4,
            title: String::from("PiPuck"),
            content: format!("{:?}", pipuck),
            // the actions depend on the state of the drone
            // the action part of the message must contain
            // the uuid, action name, and optionally arguments
            actions: vec![], //pipuck.actions(),
        };
        cards.insert(pipuck.uuid.to_string(), card);
    }
    cards
}