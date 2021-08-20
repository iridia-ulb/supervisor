use anyhow::Context;
use headers::HeaderMapExt;
use std::{net::SocketAddr, time::Duration};

use tokio_stream::{StreamMap, wrappers::{BroadcastStream, errors::BroadcastStreamRecvError}};
use futures::{FutureExt, SinkExt, StreamExt, TryFutureExt, TryStreamExt, stream::FuturesUnordered};
use tokio::{self, sync::{broadcast, mpsc, oneshot}, time::timeout};

use warp::{Filter};
use warp::reply::Response;

use crate::{
    arena,
    optitrack,
    software,
    robot::drone,
    robot::pipuck,
};

pub struct Request;

// down message (from backend to the client)
// up message (from client to the backend)

/* embed the client js and wasm into this binary */
const CLIENT_WASM_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_WASM"));
const CLIENT_JS_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_JS"));

pub async fn run(
    server_addr: SocketAddr,
    mut rx: mpsc::Receiver<Request>, // maybe unused?
    arena_tx: mpsc::Sender<arena::Request>
) {
    /* start the server */
    let wasm_route = warp::path("client_bg.wasm")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_WASM_BYTES, "content-type", "application/wasm"));
    let js_route = warp::path("client.js")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_JS_BYTES, "content-type", "application/javascript"));
    let arena_channel = warp::any().map(move || arena_tx.clone());
    let socket_route = warp::path("socket")
        .and(warp::path::end())
        .and(warp::ws())
        .and(arena_channel)
        .map(|websocket: warp::ws::Ws, arena_tx| {
            websocket.on_upgrade(move |socket| handle_client(socket, arena_tx))
        });
    let static_route = warp::get()
        .and(static_dir::static_dir!("client/public/"));
    warp::serve(js_route.or(wasm_route).or(socket_route).or(static_route))
        .run(server_addr).await   
}

async fn handle_client(
    ws: warp::ws::WebSocket,
    arena_tx: mpsc::Sender<arena::Request>
) {
    /* subscribe to drone updates and map them to websocket messages */
    let drone_updates = match subscribe_drone_updates(&arena_tx).await {
        Ok(stream) => stream
            .filter_map(|(id, update)| async {
                match update {
                    Ok(update) => {
                        Some(shared::DownMessage::UpdateDrone(id, update))
                    }
                    Err(BroadcastStreamRecvError::Lagged(count)) => {
                        log::warn!("Client missed {} messages for {}", count, id);
                        None
                    }
                }
            })
            .map(|message| bincode::serialize(&message)
                .context("Could not serialize drone message"))
            .map_ok(|encoded| warp::ws::Message::binary(encoded)),
        Err(error) => {
            log::error!("Could not initialize client: {}", error);
            return;
        }
    };
    /* subscribe to Pi-Puck updates and map them to websocket messages */
    let pipuck_updates = match subscribe_pipuck_updates(&arena_tx).await {
        Ok(stream) => stream
            .filter_map(|(id, update)| async {
                match update {
                    Ok(update) => {
                        Some(shared::DownMessage::UpdatePiPuck(id, update))
                    }
                    Err(BroadcastStreamRecvError::Lagged(count)) => {
                        log::warn!("Client missed {} messages for {}", count, id);
                        None
                    }
                }
            })
            .map(|message| bincode::serialize(&message)
                .context("Could not serialize Pi-Puck message"))
            .map_ok(|encoded| warp::ws::Message::binary(encoded)),
        Err(error) => {
            log::error!("Could not initialize client: {}", error);
            return;
        }
    };
    /* response to client requests and forward updates to client */
    tokio::pin!(pipuck_updates);
    tokio::pin!(drone_updates);
    let (mut websocket_tx, mut websocket_rx) = ws.split();


    loop {
        tokio::select! {
            /* requests from client */
            Some(rx) = websocket_rx.next() => match rx {
                Ok(message) => {
                    if message.is_close() {
                        break;
                    }
                    let message = bincode::deserialize::<shared::UpMessage>(message.as_bytes());
                    // TODO
                    log::info!("{:?}", message);
                }
                Err(error) => {
                    log::warn!("{}", error);
                }
            },
            /* send pipuck updates to client */
            Some(result) = pipuck_updates.next() => {
                match result {
                    Ok(message) => {
                        if let Err(error) = websocket_tx.send(message).await {
                            log::error!("Could not send message to client: {}", error);
                        }
                    },
                    Err(error) => log::error!("{}", error),
                }
            },
            /* send drone updates to client */
            Some(result) = drone_updates.next() => match result {
                Ok(message) => {
                    if let Err(error) = websocket_tx.send(message).await {
                        log::error!("Could not send message to client: {}", error);
                    }
                },
                Err(error) => log::error!("{}", error),                
            }
        }
    }
}

async fn subscribe_drone_updates(
    arena_tx: &mpsc::Sender<arena::Request>
) -> anyhow::Result<StreamMap<String, BroadcastStream<drone::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Request::GetDroneIds(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not request drone identifers"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get drone identifers")))
        .and_then(|drone_ids| drone_ids.into_iter()
            .map(|drone_id| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let request = drone::Request::Subscribe(callback_tx);
                arena_tx.send(arena::Request::ForwardDroneRequest(drone_id.clone(), request))
                    .map_err(|_| anyhow::anyhow!("Could not request drone updates"))
                    .and_then(|_| callback_rx
                        .map(|result| result.context("Could not subscribe to drone updates"))
                        .map_ok(|receiver| (drone_id, BroadcastStream::new(receiver))))
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>()
        ).await?;
    let mut drone_update_stream_map = StreamMap::new();
    for (id, update_stream) in update_streams {
        drone_update_stream_map.insert(id, update_stream);
    }
    Ok(drone_update_stream_map)
}

async fn subscribe_pipuck_updates(
    arena_tx: &mpsc::Sender<arena::Request>
) -> anyhow::Result<StreamMap<String, BroadcastStream<pipuck::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Request::GetPiPuckIds(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not request Pi-Puck identifers"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get Pi-Puck identifers")))
        .and_then(|pipuck_ids| pipuck_ids.into_iter()
            .map(|pipuck_id| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let request = pipuck::Request::Subscribe(callback_tx);
                arena_tx.send(arena::Request::ForwardPiPuckRequest(pipuck_id.clone(), request))
                    .map_err(|_| anyhow::anyhow!("Could not request Pi-Puck updates"))
                    .and_then(|_| callback_rx
                        .map(|result| result.context("Could not subscribe to Pi-Puck updates"))
                        .map_ok(|receiver| (pipuck_id, BroadcastStream::new(receiver))))
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>()
        ).await?;
    let mut pipuck_update_stream_map = StreamMap::new();
    for (id, update_stream) in update_streams {
        pipuck_update_stream_map.insert(id, update_stream);
    }
    Ok(pipuck_update_stream_map)
}

// the logic behind UI has changed significantly. Updates should now be sent only when there is something to update
// this means that the drone actor itself should instigate the update, perhaps by sending a message.
// actually, what we need is bidirectional communication, the webui needs to send messages to actors
// (execute command, send update), but it also needs to recieve the updates and send them back to the client

// UpMsgRequest<UpMsg> is message from the client such as UpMsg::Refresh or UpMesg::DroneExecuteAction
// UpMsg::Refresh can use local state, i.e., we keep a Vec<DroneStatus> etc in this module which is updated by the actors
// UpMsg::DroneExecuteAction needs to forwarded back to the actor, hence, we need `mpsc::Sender<arena::Request>` inside
// up_message_handler

// "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
fn generate_image_node(mime: &str, data: &[u8], style: &str) -> String {
    let data = base64::encode(data);
    let download = "save_frame(this.src.replace(/^data:image\\/[^;]+/, 'data:application/octet-stream'));";
    format!("<img src=\"data:{};base64,{}\" style=\"{}\" onclick=\"{}\" />", mime, data, style, download)
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

// pub async fn run(ws: ws::WebSocket,
//                  arena_request_tx: mpsc::Sender<arena::Request>) {
//     log::info!("Client connected");
//     /* split the socket into a sender and receive of messages */
//     let (websocket_tx, mut websocket_rx) = ws.split();

//     // TODO is this multiplexing necessary?
//     let (tx, rx) = mpsc::channel(32);
//     let rx_stream = ReceiverStream::new(rx);

//     // TODO is it desirable to spawn here?
//     tokio::task::spawn(rx_stream.forward(websocket_tx).map(|result| {
//         if let Err(error) = result {
//             log::error!("Sending data over WebSocket failed: {}", error);
//         }
//     }));

//     /* this loop is update task for a webui client */
//     while let Some(data) = websocket_rx.next().await {
//         let request : ws::Message = match data {
//             Ok(request) => request,
//             Err(error) => {
//                 log::error!("websocket receive failed: {}", error);
//                 break;
//             }
//         };
//         if let Ok(request) = request.to_str() {
//             /*
//             let t1 = Request::Upload{ target: "irobot".to_owned(), filename: "control.lua".to_owned(), data: "4591345879dsfsd908g".to_owned()};
//             let t2 = Request::PiPuck{ action: pipuck::Action::RpiReboot, uuid: uuid::Uuid::new_v4()};
//             let t3 = Request::Update{ tab: "Connections".to_owned() };
//             eprintln!("t1 = {}", serde_json::to_string(&t1).unwrap());
//             eprintln!("t2 = {}", serde_json::to_string(&t2).unwrap());
//             eprintln!("t3 = {}", serde_json::to_string(&t3).unwrap());
//             */
//             if let Ok(action) = serde_json::from_str::<Request>(request) {
//                 match action {
//                     Request::Arena{action, ..} => {
//                         let request = arena::Request::Execute(action);
//                         if let Err(error) = arena_request_tx.send(request).await {
//                             log::error!("Could not execute action on arena: {}", error);
//                         }
//                     },
//                     Request::Drone{uuid, action} => {
//                         let request = arena::Request::ForwardDroneAction(uuid, action);
//                         if let Err(error) = arena_request_tx.send(request).await {
//                             log::error!("Could not forward drone action to arena: {}", error);
//                         }
//                     },
//                     Request::PiPuck{uuid, action} => {
//                         let request = arena::Request::ForwardPiPuckAction(uuid, action);
//                         if let Err(error) = arena_request_tx.send(request).await {
//                             log::error!("Could not forward Pi-Puck action to arena: {}", error);
//                         }
//                     },
//                     Request::Update{tab} => {
//                         let result = match &tab[..] {
//                             "Connections" => connections_tab(&arena_request_tx).await,
//                             "Experiment" => experiment_tab(&arena_request_tx).await,
//                             "Optitrack" => optitrack_tab().await,
//                             _ => Err(Error::BadRequest),
//                         };
//                         let reply = match result {
//                             Ok(cards) => Reply { title: tab, cards },
//                             Err(error) => {
//                                 let error_message = format!("{}", error);
//                                 let card = Card {
//                                     uuid: uuid::Uuid::new_v3(&NAMESPACE_ERROR, error_message.as_bytes()),
//                                     span: 4,
//                                     title: "Error".to_owned(),
//                                     content: vec![Content::Text(error_message)],
//                                     actions: vec![],
//                                 };
//                                 Reply { title: tab, cards: vec![ card ] }
//                             }
//                         };
//                         match serde_json::to_string(&reply) {
//                             Ok(content) => {
//                                 let message = Ok(ws::Message::text(content));
//                                 if let Err(_) = tx.send(message).await {
//                                     log::error!("Could not reply to client");
//                                 }
//                             },
//                             Err(_) => log::error!("Could not serialize reply"),
//                         }
//                     },
//                     Request::Software{action, uuid, file} => {
//                         match action {
//                             software::Action::Upload => {
//                                 let file = file.and_then(|(name, content)| {
//                                     match content.split(',').tuples::<(_,_)>().next() {
//                                         Some((_, data)) => {
//                                             match base64::decode(data) {
//                                                 Ok(data) => Some((name, data)),
//                                                 Err(error) => {
//                                                     log::error!("Could not decode {}: {}", name, error);
//                                                     None
//                                                 }
//                                             }
//                                         },
//                                         None => None
//                                     }
//                                 });
//                                 if let Some((filename, contents)) = file {
//                                     if uuid == *UUID_ARENA_DRONES {
//                                         let request = arena::Request::AddDroneSoftware(filename, contents);
//                                         if let Err(error) = arena_request_tx.send(request).await {
//                                             log::error!("Could not add drone software: {}", error);
//                                         }
//                                     }
//                                     else if uuid == *UUID_ARENA_PIPUCKS {
//                                         let request = arena::Request::AddPiPuckSoftware(filename, contents);
//                                         if let Err(error) = arena_request_tx.send(request).await {
//                                             log::error!("Could not add Pi-Puck software: {}", error);
//                                         }
//                                     }
//                                     else {
//                                         log::error!("Target {} does not support adding software", uuid);
//                                     }
//                                 }
//                             }
//                             software::Action::Clear => {
//                                 if uuid == *UUID_ARENA_DRONES {
//                                     let request = arena::Request::ClearDroneSoftware;
//                                     if let Err(error) = arena_request_tx.send(request).await {
//                                         log::error!("Could not clear drone software: {}", error);
//                                     }
//                                 }
//                                 else if uuid == *UUID_ARENA_PIPUCKS {
//                                     let request = arena::Request::ClearPiPuckSoftware;
//                                     if let Err(error) = arena_request_tx.send(request).await {
//                                         log::error!("Could not clear Pi-Puck software: {}", error);
//                                     }
//                                 }
//                                 else {
//                                     log::error!("Target {} does not support clearing software", uuid);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//             else {
//                 log::error!("Could not deserialize request");
//             }
//         }
//     }
//     log::info!("Client disconnected");
// }

// async fn experiment_tab(arena_request_tx: &mpsc::Sender<arena::Request>) -> Result<Cards> {
//     let mut cards = Cards::default();
//     /* check pipuck software */
//     let (check_pipuck_software_callback_tx, check_pipuck_software_callback_rx) =
//         oneshot::channel();
//     let check_pipuck_software_request = 
//         arena::Request::CheckPiPuckSoftware(check_pipuck_software_callback_tx);
//     arena_request_tx
//         .send(check_pipuck_software_request).await
//         .map_err(|_| Error::ArenaRequestError)?;
//     let (pipuck_software_checksums, pipuck_software_check) = check_pipuck_software_callback_rx.await
//         .map_err(|_| Error::ArenaResponseError)?;
//     /* check drone software */
//     let (check_drone_software_callback_tx, check_drone_software_callback_rx) =
//         oneshot::channel();
//     let check_drone_software_request = 
//         arena::Request::CheckDroneSoftware(check_drone_software_callback_tx);
//     arena_request_tx
//         .send(check_drone_software_request).await
//         .map_err(|_| Error::ArenaRequestError)?;
//     let (drone_software_checksums, drone_software_check) = check_drone_software_callback_rx.await
//         .map_err(|_| Error::ArenaResponseError)?;
//     /* get actions */
//     let (get_actions_callback_tx, get_actions_callback_rx) = oneshot::channel();
//     let get_actions_request = arena::Request::GetActions(get_actions_callback_tx);
//     arena_request_tx
//         .send(get_actions_request).await
//         .map_err(|_| Error::ArenaRequestError)?;
//     let actions = get_actions_callback_rx.await
//         .map_err(|_| Error::ArenaResponseError)?;

//     let card = Card {
//         uuid: UUID_ARENA_DRONES.clone(),
//         span: 4,
//         title: "Drone Configuration".to_owned(),
//         content: vec![
//             Content::Text("Control Software".to_owned()),
//             Content::Table {
//                 header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
//                 rows: drone_software_checksums
//                     .into_iter()
//                     .map(|(filename, checksum)| vec![filename, format!("{:x}", checksum)])
//                     .collect::<Vec<_>>()
//             },
//             Content::Text(match drone_software_check {
//                 Ok(_) => format!("{} Configuration valid", OK_ICON),
//                 Err(error) => format!("{} {}", ERROR_ICON, error),
//             }),
//         ],
//         actions: vec![software::Action::Upload, software::Action::Clear]
//             .into_iter()
//             .map(Action::Software)
//             .collect(),
//     };
//     cards.push(card);
//     let card = Card {
//         uuid: UUID_ARENA_PIPUCKS.clone(),
//         span: 4,
//         title: "Pi-Puck Configuration".to_owned(),
//         content: vec![
//             Content::Text("Control Software".to_owned()),
//             Content::Table {
//                 header: vec!["File".to_owned(), "MD5 Checksum".to_owned()],
//                 rows: pipuck_software_checksums
//                     .into_iter()
//                     .map(|(filename, checksum)| vec![filename, format!("{:x}", checksum)])
//                     .collect::<Vec<_>>()
//             },
//             Content::Text(match pipuck_software_check {
//                 Ok(_) => format!("{} Configuration valid", OK_ICON),
//                 Err(error) => format!("{} {}", ERROR_ICON, error),
//             }),
//         ],
//         actions: vec![software::Action::Upload, software::Action::Clear]
//             .into_iter().map(Action::Software).collect(),
//     };
//     cards.push(card);
//     let card = Card {
//         uuid: UUID_ARENA_DASHBOARD.clone(),
//         span: 4,
//         title: String::from("Dashboard"),
//         content: vec![Content::Text(String::from("Experiment"))],
//         // the actions depend on the state of the drone
//         // the action part of the message must contain
//         // the uuid, action name, and optionally arguments
//         actions: actions.into_iter().map(Action::Arena).collect(), // start/stop experiment
//     };
//     cards.push(card);
//     Ok(cards)
// }

// async fn optitrack_tab() -> Result<Cards> {
//     let mut cards = Cards::default();
//     let update = timeout(Duration::from_millis(100), optitrack::once());
//     if let Ok(frame_of_data) = update.await.map_err(|_| Error::OptitrackTimeoutError)? {
//         for rigid_body in frame_of_data.rigid_bodies {
//             let position = format!("x = {:.3}, y = {:.3}, z = {:.3}",
//                 rigid_body.position.x,
//                 rigid_body.position.y,
//                 rigid_body.position.z);
//             let orientation = format!("w = {:.3}, x = {:.3}, y = {:.3}, z = {:.3}",
//                 rigid_body.orientation.w,
//                 rigid_body.orientation.vector().x,
//                 rigid_body.orientation.vector().y,
//                 rigid_body.orientation.vector().z);
//             let card = Card {
//                 uuid: uuid::Uuid::new_v3(&NAMESPACE_OPTITRACK, &rigid_body.id.to_be_bytes()),
//                 span: 3,
//                 title: format!("Rigid body {}", rigid_body.id),
//                 content: vec![Content::Table {
//                     header: vec!["Position".to_owned(), "Orientation".to_owned()],
//                     rows: vec![vec![position, orientation]]
//                 }],
//                 // the actions depend on the state of the drone
//                 // the action part of the message must contain
//                 // the uuid, action name, and optionally arguments
//                 actions: vec![], // start/stop experiment
//             };
//             cards.push(card);
//         }
//     }
//     Ok(cards)
// }

// async fn connections_tab(arena_request_tx: &mpsc::Sender<arena::Request>) -> Result<Cards> {
//     /* get connected Pi-Pucks */
//     let (get_pipucks_callback_tx, get_pipucks_callback_rx) = oneshot::channel();
//     let get_pipucks_request = 
//         arena::Request::GetPiPucks(get_pipucks_callback_tx);
//     arena_request_tx
//         .send(get_pipucks_request).await
//         .map_err(|_| Error::ArenaRequestError)?;
//     let pipucks = get_pipucks_callback_rx.await
//         .map_err(|_| Error::ArenaResponseError)?;
//     /* get connected drones */
//     let (get_drones_callback_tx, get_drones_callback_rx) = oneshot::channel();
//     let get_drones_request = 
//         arena::Request::GetDrones(get_drones_callback_tx);
//     arena_request_tx
//         .send(get_drones_request).await
//         .map_err(|_| Error::ArenaRequestError)?;
//     let drones = get_drones_callback_rx.await
//         .map_err(|_| Error::ArenaResponseError)?;
//     /* generate cards */
//     let mut cards = Cards::default();
//     /* generate Pi-Puck cards */
//     for (uuid, state) in pipucks.into_iter() {
//         let mut card = Card {
//             uuid: uuid,
//             span: 4,
//             title: String::from("Pi-Puck"),
//             content: vec![
//                 Content::Text("Overview".to_owned()),
//                 Content::Table {
//                     header: vec!["Unique Identifier".to_owned(), "Battery".to_owned()],
//                     rows: vec![vec![uuid.to_string(), "TODO".to_owned()]]
//                 },
//                 Content::Text("Connectivity".to_owned()),
//                 Content::Table {
//                     header: vec!["Device".to_owned(), "IP Address".to_owned(), "Signal Strength".to_owned()],
//                     rows: vec![
//                         vec![
//                             "Raspberry Pi".to_owned(),
//                             state.rpi.0.to_string(),
//                             format!("{}", match state.rpi.1 + 90 {
//                                 25..=49  => WIFI2_IMG,
//                                 50..=74  => WIFI3_IMG,
//                                 75..=100 => WIFI4_IMG,
//                                 _ => WIFI1_IMG,
//                             })
//                         ]
//                     ]
//                 }
//             ],
//             actions: state.actions.into_iter().map(Action::PiPuck).collect(),
//         };
//         if state.cameras.len() > 0 {
//             let camera_frames = state.cameras.into_iter()
//                 .map(|data| {
//                     generate_image_node("image/jpg", &data, "width:calc(50% - 10px);padding:5px;")
//                 })
//                 .collect::<String>();
//             card.content.push(Content::Text(camera_frames));
//         }
//         if let Some(kernel_messages) = state.kernel_messages {
//             let data = base64::encode(kernel_messages.as_bytes());
//             card.content.push(Content::Download { data, filename: "kernel_messages.txt".to_owned() } );
//         }
//         cards.push(card);
//     }
//     /* generate drone cards */
//     for (uuid, state) in drones.into_iter() {
//         let mut content = vec![
//             Content::Text("Overview".to_owned()),
//             Content::Table {
//                 header: vec!["Unique Identifier".to_owned(), "Battery".to_owned()],
//                 rows: vec![
//                     vec![
//                         uuid.to_string(),
//                         match state.battery_remaining {
//                             25..=49  => BATT2_IMG,
//                             50..=74  => BATT3_IMG,
//                             75..=100 => BATT4_IMG,
//                             _ => BATT1_IMG,
//                         }.to_owned()
//                     ]
//                 ]
//             },
//             Content::Text("Connectivity".to_owned()),
//             Content::Table {
//                 header: vec!["Device".to_owned(), "IP Address".to_owned(), "Signal Strength".to_owned()],
//                 rows: vec![
//                     vec![
//                         "Xbee".to_owned(),
//                         state.xbee.0.to_string(),
//                         format!("{}", match state.xbee.1 {
//                             25..=49  => WIFI2_IMG,
//                             50..=74  => WIFI3_IMG,
//                             75..=100 => WIFI4_IMG,
//                             _ => WIFI1_IMG,
//                         })
//                     ],
//                 ]
//             },
//             Content::Text("Sensors and actuators".to_owned()),
//             Content::Table {
//                 header: vec!["Device".to_owned(), "Location".to_owned()],
//                 rows: state.devices.into_iter().map(|(left, right)| vec![left, right]).collect()
//             },
//         ];
//         if let Some(upcore) = state.upcore {
//             let upcore = vec![
//                 "UP Core".to_owned(),
//                 upcore.0.to_string(),
//                 match upcore.1 + 90 {
//                     25..=49  => WIFI2_IMG,
//                     50..=74  => WIFI3_IMG,
//                     75..=100 => WIFI4_IMG,
//                     _ => WIFI1_IMG,
//                 }.to_owned()
//             ];
//             /* the third index (fourth entry) should be the connectivity table */
//             if let Some(Content::Table{rows, ..}) = content.get_mut(3) {
//                 rows.push(upcore);
//             }
//         }
//         if let Some(kernel_messages) = state.kernel_messages {
//             let data = base64::encode(kernel_messages.as_bytes());
//             content.push(Content::Download { data, filename: "kernel_messages.txt".to_owned() } );
//         }
//         let mut card = Card {
//             uuid: uuid,
//             span: 4,
//             title: String::from("Drone"),
//             content: content,
//             actions: state.actions.into_iter().map(Action::Drone).collect(),
//         };
//         if state.cameras.len() > 0 {
//             let camera_frames = state.cameras.into_iter()
//                 .map(|data| {
//                     generate_image_node("image/jpg", &data, "width:calc(50% - 10px);padding:5px;")
//                 })
//                 .collect::<String>();
//             card.content.push(Content::Text(camera_frames));
//         }
//         cards.push(card);
//     }
//     Ok(cards)
// }