use warp::ws;

use std::{
    time::Duration
};

use futures::{FutureExt, StreamExt};

use tokio::{
    sync::mpsc,
    time::timeout,
};

use regex::Regex;

use crate::{
    experiment,
    optitrack,
    software,
    robot::{self, Robot, Identifiable},
};

use serde::{Deserialize, Serialize};

use log;

use itertools::Itertools;

/// MDL HTML for icons
const OK_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:green; vertical-align: middle;\">check_circle</i>";
const ERROR_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:red; vertical-align: middle;\">error</i>";


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
    Emergency,
    Experiment {
        action: experiment::Action,
        uuid: uuid::Uuid,
    }, 
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
    Software {
        action: software::Action,
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
    Software(software::Action),
}

#[derive(Serialize, Debug)]
struct Card {
    uuid: uuid::Uuid,
    span: u8,
    title: String,
    content: Vec<Content>,
    actions: Vec<Action>,
}

type Cards = Vec<Card>;

// TODO, Reply will probably need to be wrapped in a enum soon Reply::Update, Reply::XXX
#[derive(Serialize)]
struct Reply {
    title: String,
    cards: Cards,
}

lazy_static::lazy_static! {
    /* UUIDs */
    static ref NAMESPACE_CONNECTIONS: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "connections".as_bytes());
    static ref NAMESPACE_EXPERIMENT: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "experiment".as_bytes());
    static ref NAMESPACE_OPTITRACK: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "optitrack".as_bytes());

    static ref UUID_EXPERIMENT_DRONES: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_EXPERIMENT, "drones".as_bytes());
    static ref UUID_EXPERIMENT_PIPUCKS: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_EXPERIMENT, "pipucks".as_bytes());
    static ref UUID_EXPERIMENT_DASHBOARD: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_EXPERIMENT, "dashboard".as_bytes());
    
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
    log::info!("Client connected!");
    /* split the socket into a sender and receive of messages */
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(error) = result {
            log::error!("Sending data over WebSocket failed: {}", error);
        }
    }));

    /* this loop is update task for a webui client */
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
                    Request::Experiment{action, ..} => {
                        experiment.write().await.execute(&action).await;
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
                    Request::Software{action, uuid, file} => {
                        match action {
                            software::Action::Upload => {
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
                                    if uuid == *UUID_EXPERIMENT_DRONES {
                                        let mut experiment = experiment.write().await;
                                        experiment.drone_software.add(filename, contents);
                                    }
                                    else if uuid == *UUID_EXPERIMENT_PIPUCKS {
                                        let mut experiment = experiment.write().await;
                                        experiment.pipuck_software.add(filename, contents);
                                    }
                                    else {
                                        log::error!("UUID target does not support adding control software");
                                    }
                                }
                            }
                            software::Action::Clear => {
                                if uuid == *UUID_EXPERIMENT_DRONES {
                                    let mut experiment = experiment.write().await;
                                    experiment.drone_software.clear();
                                }
                                else if uuid == *UUID_EXPERIMENT_PIPUCKS {
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
                log::error!("Could not deserialize request");
            }
        }
    }
    log::info!("Client disconnected!");
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
        uuid: UUID_EXPERIMENT_DRONES.clone(),
        span: 4,
        title: "Drone Configuration".to_owned(),
        content: vec![
            Content::Text("Control Software".to_owned()),
            Content::Table {
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
            Content::Text(match experiment.drone_software.check_config() {
                Ok(_) => format!("{} Configuration valid", OK_ICON),
                Err(error) => format!("{} {}", ERROR_ICON, error),
            }),
        ],
        actions: vec![software::Action::Upload, software::Action::Clear]
            .into_iter().map(Action::Software).collect(),
    };
    cards.push(card);
    let card = Card {
        uuid: UUID_EXPERIMENT_PIPUCKS.clone(),
        span: 4,
        title: "Pi-Puck Configuration".to_owned(),
        content: vec![
            Content::Text("Control Software".to_owned()),
            Content::Table {
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
            Content::Text(match experiment.pipuck_software.check_config() {
                Ok(_) => format!("{} Configuration valid", OK_ICON),
                Err(error) => format!("{} {}", ERROR_ICON, error),
            }),
        ],
        actions: vec![software::Action::Upload, software::Action::Clear]
            .into_iter().map(Action::Software).collect(),
    };
    cards.push(card);
    let card = Card {
        uuid: UUID_EXPERIMENT_DASHBOARD.clone(),
        span: 4,
        title: String::from("Dashboard"),
        content: vec![Content::Text(String::from("Experiment"))],
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: experiment.actions().into_iter().map(Action::Experiment).collect(), // start/stop experiment
    };
    cards.push(card);
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
                    uuid: uuid::Uuid::new_v3(&NAMESPACE_OPTITRACK, &rigid_body.id.to_be_bytes()),
                    span: 3,
                    title: format!("Rigid body {}", rigid_body.id),
                    content: vec![Content::Table {
                        header: vec!["Position".to_owned(), "Orientation".to_owned()],
                        rows: vec![vec![position, orientation]]
                    }],
                    // the actions depend on the state of the drone
                    // the action part of the message must contain
                    // the uuid, action name, and optionally arguments
                    actions: vec![], // start/stop experiment
                };
                cards.push(card);
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
                    uuid: drone.uuid.clone(),
                    span: 4,
                    title: String::from("Drone"),
                    content: vec![Content::Table {
                        header: vec!["Unique Identifier".to_owned(), "Xbee Address".to_owned(), "SSH Address".to_owned()],
                        rows: vec![vec![drone.uuid.to_string(), drone.xbee.addr.to_string(), String::from("-")]]
                    }],
                    actions: drone.actions().into_iter().map(Action::Drone).collect(),
                };
                cards.push(card);
            }
            Robot::PiPuck(pipuck) => {
                let card = Card {
                    uuid: pipuck.uuid.clone(),
                    span: 4,
                    title: String::from("Pi-Puck"),
                    content: vec![Content::Table {
                        header: vec!["Unique Identifier".to_owned(), "SSH Address".to_owned()],
                        rows: vec![vec![pipuck.uuid.to_string(), pipuck.device.addr.to_string()]]
                    }],
                    actions: pipuck.actions().into_iter().map(Action::PiPuck).collect(),
                };
                cards.push(card);
            }
        }
    }
    Reply { title: "Connections".to_owned(), cards }
}