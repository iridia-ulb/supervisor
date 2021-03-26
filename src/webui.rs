use base64::encode;
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::ws;

use std::{
    time::Duration
};

use futures::{FutureExt, StreamExt};

use tokio::{sync::{mpsc, oneshot}, time::timeout};

use regex::Regex;

use crate::{
    arena,
    optitrack,
    software,
    robot::drone,
    robot::pipuck,
};

use serde::{Deserialize, Serialize};

use log;

use itertools::Itertools;

/// MDL HTML for icons
const OK_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:green; vertical-align: middle;\">check_circle</i>";
const ERROR_ICON: &str = "<i class=\"material-icons mdl-list__item-icon\" style=\"color:red; vertical-align: middle;\">error</i>";

const WIFI0_IMG: &str = "<img src=\"images/wifi0.svg\" style=\"height:2em;padding-right:10px\" />";
const WIFI1_IMG: &str = "<img src=\"images/wifi1.svg\" style=\"height:2em;padding-right:10px\" />";
const WIFI2_IMG: &str = "<img src=\"images/wifi2.svg\" style=\"height:2em;padding-right:10px\" />";
const WIFI3_IMG: &str = "<img src=\"images/wifi3.svg\" style=\"height:2em;padding-right:10px\" />";
const WIFI4_IMG: &str = "<img src=\"images/wifi4.svg\" style=\"height:2em;padding-right:10px\" />";

// This doesn't work since it changes the current URL, disconnecting the websocket
// onclick=\"window.location.href=this.src.replace('image/png', 'image/octet-stream')\"
const TEST_IMG: &str = "<img src=\"data:image/jpg;base64,\" style=\"width:calc(50% - 10px);padding:5px;\" />";

fn generate_img_node(mime: &str, data: &[u8], style: &str) -> String {
    let data = base64::encode(data);
    format!("<img src=\"data:{};base64,{}\" style=\"{}\" />", mime, data, style)
}

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

    #[error("Could not send request to arena")]
    ArenaRequestError,
    #[error("Could not get a response from arena")]
    ArenaResponseError,

    #[error("Timed out while waiting for response from Optitrack system")]
    OptitrackTimeoutError,
}

type Result<T> = std::result::Result<T, Error>;

// TODO remove serialize
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase", tag = "type")]
enum Request {
    Arena {
        action: arena::Action,
        uuid: uuid::Uuid,
    }, 
    Drone {
        action: drone::Action,
        uuid: uuid::Uuid
    },
    PiPuck {
        action: pipuck::Action,
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
    Drone(drone::Action),
    PiPuck(pipuck::Action),
    Arena(arena::Action),
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
    static ref NAMESPACE_ARENA: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "arena".as_bytes());
    static ref NAMESPACE_OPTITRACK: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "optitrack".as_bytes());
    static ref NAMESPACE_ERROR: uuid::Uuid =
        uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_OID, "error".as_bytes());

    static ref UUID_ARENA_DRONES: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_ARENA, "drones".as_bytes());
    static ref UUID_ARENA_PIPUCKS: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_ARENA, "pipucks".as_bytes());
    static ref UUID_ARENA_DASHBOARD: uuid::Uuid =
        uuid::Uuid::new_v3(&NAMESPACE_ARENA, "dashboard".as_bytes());
    
    /* other */
    static ref IIO_CHECKS: Vec<(String, String)> =
            ["epuck-groundsensors", "epuck-motors", "epuck-leds", "epuck-rangefinders"].iter()
            .map(|dev| (String::from(*dev), format!("grep ^{} /sys/bus/iio/devices/*/name", dev)))
            .collect::<Vec<_>>();
    static ref REGEX_IIO_DEVICE: Regex = Regex::new(r"iio:device[[:digit:]]+").unwrap();
}

pub async fn run(ws: ws::WebSocket,
                 arena_request_tx: mpsc::UnboundedSender<arena::Request>) {
    log::info!("Client connected");
    /* split the socket into a sender and receive of messages */
    let (websocket_tx, mut websocket_rx) = ws.split();

    // TODO is this multiplexing necessary?
    let (tx, rx) = mpsc::unbounded_channel();
    let rx_stream = UnboundedReceiverStream::new(rx);

    // TODO is it desirable to spawn here?
    tokio::task::spawn(rx_stream.forward(websocket_tx).map(|result| {
        if let Err(error) = result {
            log::error!("Sending data over WebSocket failed: {}", error);
        }
    }));

    /* this loop is update task for a webui client */
    while let Some(data) = websocket_rx.next().await {
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
                    Request::Arena{action, ..} => {
                        let request = arena::Request::Execute(action);
                        if let Err(error) = arena_request_tx.send(request) {
                            log::error!("Could not execute action on arena: {}", error);
                        }
                    },
                    Request::Drone{uuid, action} => {
                        let request = arena::Request::ForwardDroneAction(uuid, action);
                        if let Err(error) = arena_request_tx.send(request) {
                            log::error!("Could not forward drone action to arena: {}", error);
                        }
                    },
                    Request::PiPuck{uuid, action} => {
                        let request = arena::Request::ForwardPiPuckAction(uuid, action);
                        if let Err(error) = arena_request_tx.send(request) {
                            log::error!("Could not forward Pi-Puck action to arena: {}", error);
                        }
                    },
                    Request::Update{tab} => {
                        let result = match &tab[..] {
                            "Connections" => connections_tab(&arena_request_tx).await,
                            "Experiment" => experiment_tab(&arena_request_tx).await,
                            "Optitrack" => optitrack_tab().await,
                            _ => Err(Error::BadRequest),
                        };
                        let reply = match result {
                            Ok(cards) => Reply { title: tab, cards },
                            Err(error) => {
                                let error_message = format!("{}", error);
                                let card = Card {
                                    uuid: uuid::Uuid::new_v3(&NAMESPACE_ERROR, error_message.as_bytes()),
                                    span: 4,
                                    title: "Error".to_owned(),
                                    content: vec![Content::Text(error_message)],
                                    actions: vec![],
                                };
                                Reply { title: tab, cards: vec![ card ] }
                            }
                        };
                        match serde_json::to_string(&reply) {
                            Ok(content) => {
                                let message = Ok(ws::Message::text(content));
                                if let Err(_) = tx.send(message) {
                                    log::error!("Could not reply to client");
                                }
                            },
                            Err(_) => log::error!("Could not serialize reply"),
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
                                    if uuid == *UUID_ARENA_DRONES {
                                        let request = arena::Request::AddDroneSoftware(filename, contents);
                                        if let Err(error) = arena_request_tx.send(request) {
                                            log::error!("Could not add drone software: {}", error);
                                        }
                                    }
                                    else if uuid == *UUID_ARENA_PIPUCKS {
                                        let request = arena::Request::AddPiPuckSoftware(filename, contents);
                                        if let Err(error) = arena_request_tx.send(request) {
                                            log::error!("Could not add Pi-Puck software: {}", error);
                                        }
                                    }
                                    else {
                                        log::error!("Target {} does not support adding software", uuid);
                                    }
                                }
                            }
                            software::Action::Clear => {
                                if uuid == *UUID_ARENA_DRONES {
                                    let request = arena::Request::ClearDroneSoftware;
                                    if let Err(error) = arena_request_tx.send(request) {
                                        log::error!("Could not clear drone software: {}", error);
                                    }
                                }
                                else if uuid == *UUID_ARENA_PIPUCKS {
                                    let request = arena::Request::ClearPiPuckSoftware;
                                    if let Err(error) = arena_request_tx.send(request) {
                                        log::error!("Could not clear Pi-Puck software: {}", error);
                                    }
                                }
                                else {
                                    log::error!("Target {} does not support clearing software", uuid);
                                }
                            }
                        }
                    }
                }
            }
            else {
                log::error!("Could not deserialize request");
            }
        }
    }
    log::info!("Client disconnected");
}

async fn experiment_tab(arena_request_tx: &mpsc::UnboundedSender<arena::Request>) -> Result<Cards> {
    let mut cards = Cards::default();
    /* check pipuck software */
    let (check_pipuck_software_callback_tx, check_pipuck_software_callback_rx) =
        oneshot::channel();
    let check_pipuck_software_request = 
        arena::Request::CheckPiPuckSoftware(check_pipuck_software_callback_tx);
    arena_request_tx
        .send(check_pipuck_software_request)
        .map_err(|_| Error::ArenaRequestError)?;
    let (pipuck_software_checksums, pipuck_software_check) = check_pipuck_software_callback_rx.await
        .map_err(|_| Error::ArenaResponseError)?;
    /* check drone software */
    let (check_drone_software_callback_tx, check_drone_software_callback_rx) =
        oneshot::channel();
    let check_drone_software_request = 
        arena::Request::CheckDroneSoftware(check_drone_software_callback_tx);
    arena_request_tx
        .send(check_drone_software_request)
        .map_err(|_| Error::ArenaRequestError)?;
    let (drone_software_checksums, drone_software_check) = check_drone_software_callback_rx.await
        .map_err(|_| Error::ArenaResponseError)?;
    /* get actions */
    let (get_actions_callback_tx, get_actions_callback_rx) = oneshot::channel();
    let get_actions_request = arena::Request::GetActions(get_actions_callback_tx);
    arena_request_tx
        .send(get_actions_request)
        .map_err(|_| Error::ArenaRequestError)?;
    let actions = get_actions_callback_rx.await
        .map_err(|_| Error::ArenaResponseError)?;

    let card = Card {
        uuid: UUID_ARENA_DRONES.clone(),
        span: 4,
        title: "Drone Configuration".to_owned(),
        content: vec![
            Content::Text("Control Software".to_owned()),
            Content::Table {
                header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
                rows: drone_software_checksums
                    .into_iter()
                    .map(|(filename, checksum)| vec![filename, format!("{:x}", checksum)])
                    .collect::<Vec<_>>()
            },
            Content::Text(match drone_software_check {
                Ok(_) => format!("{} Configuration valid", OK_ICON),
                Err(error) => format!("{} {}", ERROR_ICON, error),
            }),
        ],
        actions: vec![software::Action::Upload, software::Action::Clear]
            .into_iter()
            .map(Action::Software)
            .collect(),
    };
    cards.push(card);
    let card = Card {
        uuid: UUID_ARENA_PIPUCKS.clone(),
        span: 4,
        title: "Pi-Puck Configuration".to_owned(),
        content: vec![
            Content::Text("Control Software".to_owned()),
            Content::Table {
                header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
                rows: pipuck_software_checksums
                    .into_iter()
                    .map(|(filename, checksum)| vec![filename, format!("{:x}", checksum)])
                    .collect::<Vec<_>>()
            },
            Content::Text(match pipuck_software_check {
                Ok(_) => format!("{} Configuration valid", OK_ICON),
                Err(error) => format!("{} {}", ERROR_ICON, error),
            }),
        ],
        actions: vec![software::Action::Upload, software::Action::Clear]
            .into_iter().map(Action::Software).collect(),
    };
    cards.push(card);
    let card = Card {
        uuid: UUID_ARENA_DASHBOARD.clone(),
        span: 4,
        title: String::from("Dashboard"),
        content: vec![Content::Text(String::from("Experiment"))],
        // the actions depend on the state of the drone
        // the action part of the message must contain
        // the uuid, action name, and optionally arguments
        actions: actions.into_iter().map(Action::Arena).collect(), // start/stop experiment
    };
    cards.push(card);
    Ok(cards)
}

async fn optitrack_tab() -> Result<Cards> {
    let mut cards = Cards::default();
    let update = timeout(Duration::from_millis(100), optitrack::once());
    if let Ok(frame_of_data) = update.await.map_err(|_| Error::OptitrackTimeoutError)? {
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
    Ok(cards)
}

async fn connections_tab(arena_request_tx: &mpsc::UnboundedSender<arena::Request>) -> Result<Cards> {
    /* get connected Pi-Pucks */
    let (get_pipucks_callback_tx, get_pipucks_callback_rx) = oneshot::channel();
    let get_pipucks_request = 
        arena::Request::GetPiPucks(get_pipucks_callback_tx);
    arena_request_tx
        .send(get_pipucks_request)
        .map_err(|_| Error::ArenaRequestError)?;
    let pipucks = get_pipucks_callback_rx.await
        .map_err(|_| Error::ArenaResponseError)?;
    /* get connected drones */
    let (get_drones_callback_tx, get_drones_callback_rx) = oneshot::channel();
    let get_drones_request = 
        arena::Request::GetDrones(get_drones_callback_tx);
    arena_request_tx
        .send(get_drones_request)
        .map_err(|_| Error::ArenaRequestError)?;
    let drones = get_drones_callback_rx.await
        .map_err(|_| Error::ArenaResponseError)?;
    /* generate cards */
    let mut cards = Cards::default();
    /* generate Pi-Puck cards */
    for (uuid, state) in pipucks.into_iter() {
        let mut card = Card {
            uuid: uuid,
            span: 4,
            title: String::from("Pi-Puck"),
            content: vec![Content::Table {
                header: vec!["Unique Identifier".to_owned(), "Raspberry Pi".to_owned()],
                rows: vec![vec![uuid.to_string(), format!("{} {}", match state.rpi.1 + 90 {
                    15..=29  => WIFI1_IMG,
                    30..=44  => WIFI2_IMG,
                    45..=59  => WIFI3_IMG,
                    60..=100 => WIFI4_IMG,
                    _ => WIFI0_IMG,
                }, state.rpi.0)]]
            }],
            actions: state.actions.into_iter().map(Action::PiPuck).collect(),
        };
        if let Some(data) = state.camera {
            let image = generate_img_node("image/jpg", &data, "width:calc(50% - 10px);padding:5px;");
            card.content.push(Content::Text(image));
        }
        cards.push(card);       
    }
    /* generate drone cards */
    for (uuid, state) in drones.into_iter() {
        let card = Card {
            uuid: uuid,
            span: 4,
            title: String::from("Drone"),
            content: vec![Content::Table {
                header: vec!["Unique Identifier".to_owned(), "Xbee".to_owned(), "UP Core".to_owned()],
                rows: vec![
                    vec![
                        uuid.to_string(),
                        format!("{} {}", match state.xbee.1 {
                            15..=29  => WIFI1_IMG,
                            30..=44  => WIFI2_IMG,
                            45..=59  => WIFI3_IMG,
                            60..=100 => WIFI4_IMG,
                            _ => WIFI0_IMG,
                        }, state.xbee.0),
                        state.upcore.map_or_else(|| "-".to_owned(), |upcore| {
                            format!("{} {}", match upcore.1 + 90 {
                                15..=29  => WIFI1_IMG,
                                30..=44  => WIFI2_IMG,
                                45..=59  => WIFI3_IMG,
                                60..=100 => WIFI4_IMG,
                                _ => WIFI0_IMG,
                            }, upcore.0)
                        })
                    ]
                ]
            }],
            actions: state.actions.into_iter().map(Action::Drone).collect(),
        };
        cards.push(card);
    }
    Ok(cards)
}