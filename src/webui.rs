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

#[derive(Serialize, Debug)]
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
    // view
    Update(String),
    // action, robot uuid
    Drone(drone::Action, uuid::Uuid),
    // action, robot uuid
    PiPuck(pipuck::Action, uuid::Uuid),
    // action, vec of robot uuids
    Experiment(experiment::Action),

    //
    Emergency,
}

/// while it is possible to put a trait bound on this struct as follows
/// ```
/// struct Card<T: Serialize> {
/// ...
/// actions: Vec<T>
/// }
/// ```
/// This makes a Card<action::PiPuck> different from Card<action::Drone>
/// which results in not being able to put them into the same collection
/// that's why we use dynamic dispatch here
#[derive(Serialize)]
struct Card {
    span: u8,
    title: String,
    content: Content,
    actions: Vec<Box<dyn erased_serde::Serialize + Send>>
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
                    Request::Drone(action, uuid) => {
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
        actions: vec![Box::new(6)],
    };
    cards.insert(String::from("Drone"), card);

    let card = Card {
        span: 6,
        title: String::from("Pi-Puck Configuration"),
        content: Content::Text(String::from("Drone")),
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: vec![Box::new('a')],
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
            actions: drone.actions().drain(..)
                .map(|action| Box::new(action) as Box<dyn erased_serde::Serialize + Send>)
                .collect(),
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
            actions: pipuck.actions().drain(..)
                .map(|action| Box::new(action) as Box<dyn erased_serde::Serialize + Send>)
                .collect(),
        };
        cards.insert(pipuck.uuid.to_string(), card);
    }
    cards
}