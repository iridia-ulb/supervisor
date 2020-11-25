use warp::ws;

use std::collections::HashMap;

use futures::{FutureExt, StreamExt};

use tokio::{
    sync::{ mpsc },
};

use regex::Regex;

use super::{
    experiment,
    robots::{
        drone,
        pipuck,
    },
    Robots
};

use serde::{Deserialize, Serialize};

use log;

#[derive(Serialize, Debug)]
#[serde(rename_all = "lowercase")]
enum Content {
    Text(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>
    },
}

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
    Action(Action, uuid::Uuid),
    Emergency,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase", tag = "robot", content = "command")]
enum Action {
    Drone(drone::Action),
    PiPuck(pipuck::Action),
    Experiment(experiment::Action),
}

#[derive(Serialize, Debug)]
struct Card {
    span: u8,
    title: String,
    content: Content,
    actions: Vec<Action>,
}

type Cards = HashMap<String, Card>;

// TODO, Reply will probably need to be wrapped in a enum soon Reply::Update, Reply::XXX
#[derive(Serialize)]
struct Reply {
    title: String,
    cards: Cards,
}

pub async fn run(ws: ws::WebSocket, drones: Robots<drone::Drone>, pipucks: Robots<pipuck::PiPuck>) {
    // Use a counter to assign a new unique ID for this user.

    log::info!("client connected!");
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
                                Ok(Reply {
                                    title: String::from("Connections"),
                                    cards: connections(&drones, &pipucks).await
                                })
                            },
                            "diagnostics" => {
                                Ok(Reply {
                                    title: String::from("Diagnostics"),
                                    cards: diagnostics(&drones, &pipucks).await
                                })
                            },
                            "experiment" => {
                                Ok(Reply {
                                    title: String::from("Experiment"),
                                    cards: experiment(&drones, &pipucks).await
                                })
                            },
                            "optitrack" => {
                                Ok(Reply {
                                    title: String::from("Optitrack"),
                                    cards: optitrack(&drones, &pipucks).await
                                })
                            },
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
                    Request::Action(action, uuid) => {
                        match action {
                            Action::Drone(action) => {
                                let mut drones = drones.write().await;
                                if let Some(drone) = drones.iter_mut().find(|drone| drone.uuid == uuid) {
                                    drone.execute(&action);
                                }
                                else {
                                    log::warn!("Drone ({}) has disconnected", uuid)
                                }
                            },
                            Action::PiPuck(action) => {
                                let mut pipucks = pipucks.write().await;
                                if let Some(pipuck) = pipucks.iter_mut().find(|pipuck| pipuck.uuid == uuid) {
                                    pipuck.execute(&action);
                                }
                                else {
                                    log::warn!("Pi-Puck ({}) has disconnected", uuid)
                                }
                            },
                            Action::Experiment(_) => {

                            }
                        }
                    },
                    Request::Emergency => {
                        // Go to emergency mode
                    }
                }
            }
            else {
                log::error!("Cannot not deserialize message");
            }
        }
    }
    log::info!("client disconnected!");
}

async fn diagnostics(_drones: &Robots<drone::Drone>, pipucks: &Robots<pipuck::PiPuck>) -> Cards {
    lazy_static::lazy_static! {
        static ref IIO_CHECKS: Vec<(String, String)> =
            ["epuck-groundsensors", "epuck-motors", "epuck-leds", "epuck-rangefinders"].iter()
            .map(|dev| (String::from(*dev), format!("grep ^{} /sys/bus/iio/devices/*/name", dev)))
            .collect::<Vec<_>>();
        static ref REGEX_IIO_DEVICE: Regex = Regex::new(r"iio:device[[:digit:]]+").unwrap();
        static ref OK_ICON: String = 
            String::from("<i class=\"material-icons mdl-list__item-icon\" style=\"color:green;\">check_circle</i>");
        static ref ERROR_ICON: String = 
            String::from("<i class=\"material-icons mdl-list__item-icon\" style=\"color:red;\">error</i>");        
    }
    let mut cards = Cards::default();
    // TODO this should all be done asyncronously and combined with try_join. At the moment,
    // we are waiting for each robot to reply X times via SSH before moving on to the next robot
    for pipuck in pipucks.write().await.iter_mut() {
        let mut responses = Vec::with_capacity(IIO_CHECKS.len());
        for iio_check in IIO_CHECKS.iter() {
            let response = match pipuck.ssh.exec(&iio_check.1, true).await {
                Ok(response) => {
                    if let Some(response) = response {
                        if let Some(device) = REGEX_IIO_DEVICE.find(&response) {
                            let device = &response[device.start() .. device.end()];
                            vec![OK_ICON.clone(), iio_check.0.clone(), device.to_owned()]
                            
                        }
                        else {
                            vec![ERROR_ICON.clone(), iio_check.0.clone(), "Not found".to_owned()]
                        }
                    }
                    else {
                        vec![ERROR_ICON.clone(), iio_check.0.clone(), "No reply".to_owned()]
                    }
                }
                Err(err) => {
                    vec![ERROR_ICON.clone(), iio_check.0.clone(), err.to_string()]
                }
            };
            responses.push(response);
        }
  
        let header = ["Status", "Device", "Information"].iter().map(|s| {
            String::from(*s)
        }).collect::<Vec<_>>();

        

        let card = Card {
            span: 3,
            title: format!("Pi-Puck ({})", pipuck.ssh.hostname().await.unwrap_or("-".to_owned())),
            //content: Content::List(responses),
            
            content: Content::Table {
                header: header,
                rows: responses
            },
            
            // the actions depend on the state of the drone
            // the action part of the message must contain
            // the uuid, action name, and optionally arguments
            actions: Vec::new(), //pipuck.actions(),
        };
        cards.insert(pipuck.uuid.to_string(), card);
    }
    cards
}

async fn experiment(_drones: &Robots<drone::Drone>, _pipucks: &Robots<pipuck::PiPuck>) -> Cards {
    let mut cards = Cards::default();
       
    let card = Card {
        span: 6,
        title: String::from("Drone Configuration"),
        content: Content::Text(String::from("Drone")),
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: vec![],
    };
    cards.insert(String::from("Drone"), card);

    let card = Card {
        span: 6,
        title: String::from("Pi-Puck Configuration"),
        content: Content::Text(String::from("Drone")),
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: vec![],
    };
    cards.insert(String::from("Pi-Puck"), card);

    let card = Card {
        span: 12,
        title: String::from("Dashboard"),
        content: Content::Text(String::from("Drone")),
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: vec![], // start/stop experiment
    };
    cards.insert(String::from("Dashboard"), card);

    cards
}

async fn optitrack(_drones: &Robots<drone::Drone>, _pipucks: &Robots<pipuck::PiPuck>) -> Cards {
    Cards::default()
}

async fn connections(drones: &Robots<drone::Drone>, pipucks: &Robots<pipuck::PiPuck>) -> Cards {
    let mut cards = Cards::default();
    for drone in drones.read().await.iter() {

        let card = Card {
            span: 4,
            title: String::from("Drone"),
            content: Content::Table {
                header: vec!["Unique Identifier".to_owned(), "Xbee Address".to_owned(), "SSH Address".to_owned()],
                rows: vec![vec![drone.uuid.to_string(), drone.xbee.addr.to_string(), String::from("-")]]
            },

            // we need to convert actions back and forth between the JSON representation and the Rust representation
            // we could convert these actions to strings and convert them back.

            // the problem with converting them to JSON now, is that this struct itself needs to converted to JSON which gives
            // a json inside json situation that no one really wants.
            actions: drone.actions().into_iter().map(Action::Drone).collect(),
        };
        cards.insert(drone.uuid.to_string(), card);
    }
    for pipuck in pipucks.read().await.iter() {
        let card = Card {
            span: 4,
            title: String::from("Pi-Puck"),
            content: Content::Table {
                header: vec!["Unique Identifier".to_owned(), "SSH Address".to_owned()],
                rows: vec![vec![pipuck.uuid.to_string(), pipuck.ssh.addr.to_string()]]
            },
            actions: pipuck.actions().into_iter().map(Action::PiPuck).collect(),
        };
        cards.insert(pipuck.uuid.to_string(), card);
    }
    cards
}