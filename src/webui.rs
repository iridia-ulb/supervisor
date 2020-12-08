use warp::ws;

use std::{
    collections::HashMap,
    time::Duration
};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};

use tokio::{
    sync::mpsc,
    time::timeout,
};

use regex::Regex;

use crate::{
    experiment,
    optitrack,
    firmware,
    robot::{self, Robot, Identifiable},
};

use serde::{Deserialize, Serialize};

use log;

use itertools::Itertools;

/// MDL HTML for icons
// const OK_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:green;\">check_circle</i>";
const ERROR_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:red;\">error</i>";


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

//pub type Result<T> = std::result::Result<T, Error>;

// TODO remove serialize
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase", tag = "type")]
enum Request {
    Experiment(experiment::Action),
    Emergency,  
    Drone {
        action: robot::drone::Action,
        uuid: uuid::Uuid
    },
    PiPuck {
        action: robot::pipuck::Action,
        uuid: uuid::Uuid
    },
    Update {
        tab: String
    },
    Firmware {
        action: firmware::Action,
        file: Option<(String, String)>,
        uuid: uuid::Uuid
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "lowercase", tag = "type", content = "action")]
enum Action {
    Drone(robot::drone::Action),
    PiPuck(robot::pipuck::Action),
    Experiment(experiment::Action),
    Firmware(firmware::Action),
}

#[derive(Serialize, Debug)]
struct Card {
    span: u8,
    title: String,
    content: Content,
    actions: Vec<Action>,
}

type Cards = HashMap<uuid::Uuid, Card>;

// TODO, Reply will probably need to be wrapped in a enum soon Reply::Update, Reply::XXX
#[derive(Serialize)]
struct Reply {
    title: String,
    cards: Cards,
}

lazy_static::lazy_static! {
    /* UUIDs */
    static ref UUID_CONFIG: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "experiment".as_bytes());
    static ref UUID_CONFIG_DRONE: uuid::Uuid =
        uuid::Uuid::new_v3(&UUID_CONFIG, "drones".as_bytes());
    static ref UUID_CONFIG_PIPUCK: uuid::Uuid =
        uuid::Uuid::new_v3(&UUID_CONFIG, "pipucks".as_bytes());
    
    /* other */
    static ref IIO_CHECKS: Vec<(String, String)> =
            ["epuck-groundsensors", "epuck-motors", "epuck-leds", "epuck-rangefinders"].iter()
            .map(|dev| (String::from(*dev), format!("grep ^{} /sys/bus/iio/devices/*/name", dev)))
            .collect::<Vec<_>>();
    static ref REGEX_IIO_DEVICE: Regex = Regex::new(r"iio:device[[:digit:]]+").unwrap();
}

pub async fn run(ws: ws::WebSocket,
                 robots: crate::Robots,
                 experiment: crate::Experiment) {
    // Use a counter to assign a new unique ID for this user.

    log::info!("client connected!");
    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(error) = result {
            log::error!("websocket send failed: {}", error);
        }
    }));

    // this loop is basically our gui updating thread
    while let Some(data) = user_ws_rx.next().await {
        let request : ws::Message = match data {
            Ok(request) => request,
            Err(error) => {
                log::error!("websocket receive failed: {}", error);
                break;
            }
        };
        if let Ok(request) = request.to_str() {
            /*
            let t1 = Request::Upload{ target: "irobot".to_owned(), filename: "control.lua".to_owned(), data: "4591345879dsfsd908g".to_owned()};
            let t2 = Request::PiPuck{ action: pipuck::Action::RpiReboot, uuid: uuid::Uuid::new_v4()};
            let t3 = Request::Update{ tab: "Connections".to_owned() };
            eprintln!("t1 = {}", serde_json::to_string(&t1).unwrap());
            eprintln!("t2 = {}", serde_json::to_string(&t2).unwrap());
            eprintln!("t3 = {}", serde_json::to_string(&t3).unwrap());
            */
            if let Ok(action) = serde_json::from_str::<Request>(request) {
                match action {
                    Request::Emergency => {
                        // Go to emergency mode
                    },
                    Request::Experiment(action) => {
                        experiment.write().await.execute(&action);
                    },
                    // TODO: there is too much code duplication between drone and pipuck here
                    Request::Drone{action, uuid} => {
                        let mut robots = robots.write().await;
                        if let Some(robot) = robots.iter_mut().find(|robot| robot.id() == &uuid) {
                            if let Robot::Drone(drone) = robot {
                                drone.execute(&action);
                            }
                            else {
                                log::error!("Could not execute {:?}. {} is not a drone.", action, uuid);
                            }
                        }
                        else {
                            log::warn!("Could not execute {:?}, drone ({}) has disconnected", action, uuid);
                        }
                    },
                    Request::PiPuck{action, uuid} => {
                        let mut robots = robots.write().await;
                        if let Some(robot) = robots.iter_mut().find(|robot| robot.id() == &uuid) {
                            if let Robot::PiPuck(pipuck) = robot {
                                pipuck.execute(&action).await;
                            }
                            else {
                                log::error!("Could not execute {:?}. {} is not a Pi-Puck.", action, uuid);
                            }
                        }
                        else {
                            log::warn!("Could not execute {:?}, Pi-Puck ({}) has disconnected", action, uuid);
                        }
                    },
                    Request::Update{tab} => {
                        let reply = match &tab[..] {
                            "connections" => Ok(connections_tab(&robots).await),
                            "diagnostics" => Ok(diagnostics_tab(&robots).await),
                            "experiment" => Ok(experiment_tab(&robots, &experiment).await),
                            "optitrack" => Ok(optitrack_tab().await),
                            _ => Err(Error::BadRequest),
                        };
                        let result = reply
                            .and_then(|inner| {
                                serde_json::to_string(&inner).map_err(|err| Error::JsonError(err))
                            }).and_then(|inner| {
                                let message = Ok(ws::Message::text(inner));
                                tx.send(message).map_err(|_| Error::ReplyError)
                            });
                        if let Err(error) = result {
                            log::error!("Could not reply to client: {}", error);
                        }
                    },
                    Request::Firmware{action, uuid, file} => {
                        match action {
                            firmware::Action::Upload => {
                                eprintln!("{:?} {} {:?}", action, uuid, file);
                                let file = file.and_then(|(name, content)| {
                                    match content.split(',').tuples::<(_,_)>().next() {
                                        Some((_, data)) => {
                                            match base64::decode(data) {
                                                Ok(data) => Some((name, data)),
                                                Err(error) => {
                                                    log::error!("Could not decode {}: {}", name, error);
                                                    None
                                                }
                                            }
                                        },
                                        None => None
                                    }
                                });
                                if let Some((filename, contents)) = file {
                                    if uuid == *UUID_CONFIG_DRONE {
                                        let mut experiment = experiment.write().await;
                                        experiment.drone_software.add(filename, contents);
                                    }
                                    else if uuid == *UUID_CONFIG_PIPUCK {
                                        let mut experiment = experiment.write().await;
                                        experiment.pipuck_software.add(filename, contents);
                                    }
                                    else {
                                        log::error!("UUID target does not support adding control software");
                                    }
                                }
                            }
                            firmware::Action::Clear => {
                                if uuid == *UUID_CONFIG_DRONE {
                                    let mut experiment = experiment.write().await;
                                    experiment.drone_software.clear();
                                }
                                else if uuid == *UUID_CONFIG_PIPUCK {
                                    let mut experiment = experiment.write().await;
                                    experiment.pipuck_software.clear();
                                }
                                else {
                                    log::error!("UUID target does not support clearing control software");
                                }
                            }
                        }
                    },
                }
            }
            else {
                log::error!("cannot not deserialize message");
            }
        }
    }
    log::info!("client disconnected!");
}

async fn diagnostics_tab(_robots: &crate::Robots) -> Reply {
    /* hashmap of cards */
    let cards = Cards::default();
    Reply { title: "Diagnostics".to_owned(), cards }
}

async fn experiment_tab(_: &crate::Robots, experiment: &crate::Experiment) -> Reply {
    let mut cards = Cards::default();
    let experiment = experiment.read().await;
    let card = Card {
        span: 6,
        title: String::from("Drone Configuration"),
        content: Content::Table {
            header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
            rows: experiment.drone_software.0
                .iter()
                .map(|(filename, contents)| {
                    let filename = filename.to_string_lossy().into_owned();
                    let checksum = format!("{:x}", md5::compute(contents));
                    vec![filename, checksum]
                })
                .collect::<Vec<_>>()
        },
        actions: vec![firmware::Action::Upload, firmware::Action::Clear]
            .into_iter().map(Action::Firmware).collect(),
    };
    cards.insert(*UUID_CONFIG_DRONE, card);
    let card = Card {
        span: 6,
        title: String::from("Pi-Puck Configuration"),
        content: Content::Table {
            header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
            rows: experiment.pipuck_software.0
                .iter()
                .map(|(filename, contents)| {
                    let filename = filename.to_string_lossy().into_owned();
                    let checksum = format!("{:x}", md5::compute(contents));
                    vec![filename, checksum]
                })
                .collect::<Vec<_>>()
        },
        actions: vec![firmware::Action::Upload, firmware::Action::Clear]
            .into_iter().map(Action::Firmware).collect(),
    };
    cards.insert(*UUID_CONFIG_PIPUCK, card);
    let card = Card {
        span: 12,
        title: String::from("Dashboard"),
        content: Content::Text(String::from("Drone")),
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: experiment.actions().into_iter().map(Action::Experiment).collect(), // start/stop experiment
    };
    cards.insert(uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "experiment:dashboard".as_bytes()), card);
    Reply { title: "Experiment".to_owned(), cards }
}

async fn optitrack_tab() -> Reply {
    let mut cards = Cards::default();

    if let Ok(inner) = timeout(Duration::from_millis(100), optitrack::once()).await {
        if let Ok(frame_of_data) = inner {
            for rigid_body in frame_of_data.rigid_bodies {
                let position = format!("x = {:.3}, y = {:.3}, z = {:.3}",
                    rigid_body.position.x,
                    rigid_body.position.y,
                    rigid_body.position.z);
                let orientation = format!("w = {:.3}, x = {:.3}, y = {:.3}, z = {:.3}",
                    rigid_body.orientation.w,
                    rigid_body.orientation.vector().x,
                    rigid_body.orientation.vector().y,
                    rigid_body.orientation.vector().z);
                let card = Card {
                    span: 3,
                    title: format!("Rigid body {}", rigid_body.id),
                    content: Content::Table {
                        header: vec!["Position".to_owned(), "Orientation".to_owned()],
                        rows: vec![vec![position, orientation]]
                    },
                    // the actions depend on the state of the drone
                    // the action part of the message must contain
                    // the uuid, action name, and optionally arguments
                    actions: vec![], // start/stop experiment
                };
                cards.insert(uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, &rigid_body.id.to_be_bytes()), card);
            }
        }
        Reply { title: "Optitrack".to_owned(), cards }
    }
    else {
        Reply { title: "Optitrack [OFFLINE]".to_owned(), cards }
    }
}

async fn connections_tab(robots: &crate::Robots) -> Reply {
    let mut cards = Cards::default();
    for robot in robots.read().await.iter() {
        match robot {
            Robot::Drone(drone) => {
                let card = Card {
                    span: 4,
                    title: String::from("Drone"),
                    content: Content::Table {
                        header: vec!["Unique Identifier".to_owned(), "Xbee Address".to_owned(), "SSH Address".to_owned()],
                        rows: vec![vec![drone.uuid.to_string(), drone.xbee.addr.to_string(), String::from("-")]]
                    },
                    actions: drone.actions().into_iter().map(Action::Drone).collect(),
                };
                cards.insert(drone.uuid.clone(), card);
            }
            Robot::PiPuck(pipuck) => {
                let card = Card {
                    span: 4,
                    title: String::from("Pi-Puck"),
                    content: Content::Table {
                        header: vec!["Unique Identifier".to_owned(), "SSH Address".to_owned()],
                        rows: vec![vec![pipuck.uuid.to_string(), pipuck.ssh.addr.to_string()]]
                    },
                    actions: pipuck.actions().into_iter().map(Action::PiPuck).collect(),
                };
                cards.insert(pipuck.uuid.clone(), card);
            }
        }
    }
    Reply { title: "Connections".to_owned(), cards }
}