use anyhow::Context;
use headers::HeaderMapExt;
use lazy_static::__Deref;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio_stream::{StreamMap, wrappers::{BroadcastStream, errors::BroadcastStreamRecvError}};
use futures::{FutureExt, SinkExt, StreamExt, TryFutureExt, TryStreamExt, stream::{self, FuturesUnordered}};
use tokio::{self, sync::{broadcast, mpsc, oneshot}, time::timeout};

use warp::{Filter};
use warp::reply::Response;

use shared::{UpMessage, DownMessage};

use crate::{
    arena,
    optitrack,
    software,
    robot::drone,
    robot::pipuck,
};


// #[derive(Debug, Deserialize, Clone, Copy, Serialize)]
// pub enum State {
//     Standby,
//     Active,
// }

// #[derive(Clone, Debug, Deserialize, Serialize)]
// pub enum Request {
//     AddDroneSoftware(String, Vec<u8>),
//     ClearDroneSoftware,
//     AddPiPuckSoftware(String, Vec<u8>),
//     ClearPiPuckSoftware,
//     GetExperimentState,
//     StartExperiment,
//     StopExperiment,
// }

// #[derive(Debug, Deserialize, Serialize)]
// pub enum Update {
//     State(State),
//     DroneSoftware {
//         checksums: Vec<(String, String)>,
//         status: Result<(), String>
//     },
//     PiPuckSoftware {
//         checksums: Vec<(String, String)>,
//         status: Result<(), String>
//     },
// }

#[derive(Clone)]
pub enum Request {
    BroadcastDownMessage(DownMessage)
}

// down message (from backend to the client)
// up message (from client to the backend)

/* embed the client js and wasm into this binary */
const CLIENT_WASM_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_WASM"));
const CLIENT_JS_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_JS"));

pub async fn run(
    server_addr: SocketAddr,
    client_tx: broadcast::Sender<Request>,
    arena_tx: mpsc::Sender<arena::Request>
) {
    /* start the server */
    let wasm_route = warp::path("client_bg.wasm")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_WASM_BYTES, "content-type", "application/wasm"));
    let js_route = warp::path("client.js")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_JS_BYTES, "content-type", "application/javascript"));
    let arena_tx = warp::any().map(move || arena_tx.clone());
    let client_rx = warp::any().map(move || client_tx.subscribe());
    let socket_route = warp::path("socket")
        .and(warp::path::end())
        .and(warp::ws())
        .and(arena_tx)
        .and(client_rx)
        .map(|websocket: warp::ws::Ws, arena_tx, client_rx| {
            websocket.on_upgrade(move |socket| handle_client(socket, arena_tx, client_rx))
        });
    let static_route = warp::get()
        .and(static_dir::static_dir!("client/public/"));
    warp::serve(js_route.or(wasm_route).or(socket_route).or(static_route))
        .run(server_addr).await   
}

async fn handle_client(
    ws: warp::ws::WebSocket,
    arena_tx: mpsc::Sender<arena::Request>,
    mut client_rx: broadcast::Receiver<Request>
) {
    /* subscribe to drone updates and map them to websocket messages */
    let drone_updates = match subscribe_drone_updates(&arena_tx).await {
        Ok(updates) => {
            let add_drone_messages = updates.keys()
                .cloned()
                .map(|desc| DownMessage::AddDrone(desc.deref().clone()))
                .collect::<Vec<_>>();
            let update_drone_messages = updates
                .filter_map(|(desc, update)| async move {
                    match update {
                        Ok(update) => {
                            Some(DownMessage::UpdateDrone(desc.id.clone(), update))
                        }
                        Err(BroadcastStreamRecvError::Lagged(count)) => {
                            log::warn!("Client missed {} messages for {}", count, desc);
                            None
                        }
                    }
                });
            /* send the add drone messages first, then stream the drone updates */
            stream::iter(add_drone_messages).chain(update_drone_messages)
                .map(|message| bincode::serialize(&message)
                    .context("Could not serialize drone message"))
                .map_ok(|encoded| warp::ws::Message::binary(encoded))
        },
        Err(error) => {
            log::error!("Could not initialize client: {}", error);
            return;
        }
    };
    /* subscribe to pipuck updates and map them to websocket messages */
    let pipuck_updates = match subscribe_pipuck_updates(&arena_tx).await {
        Ok(updates) => {
            let add_pipuck_messages = updates.keys()
                .cloned()
                .map(|desc| DownMessage::AddPiPuck(desc.deref().clone()))
                .collect::<Vec<_>>();
            let update_pipuck_messages = updates
                .filter_map(|(desc, update)| async move {
                    match update {
                        Ok(update) => {
                            Some(DownMessage::UpdatePiPuck(desc.id.clone(), update))
                        }
                        Err(BroadcastStreamRecvError::Lagged(count)) => {
                            log::warn!("Client missed {} messages for {}", count, desc);
                            None
                        }
                    }
                });
            /* send the add drone messages first, then stream the pipuck updates */
            stream::iter(add_pipuck_messages).chain(update_pipuck_messages)
                .map(|message| bincode::serialize(&message)
                    .context("Could not serialize Pi-Puck message"))
                .map_ok(|encoded| warp::ws::Message::binary(encoded))
        },
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
            /* handle updates from arena */
            result = client_rx.recv() => match result {
                Ok(request) => match request {
                    Request::BroadcastDownMessage(down_msg) => match bincode::serialize(&down_msg) {
                        Ok(encoded) => {
                            let message = warp::ws::Message::binary(encoded);
                            if let Err(error) = websocket_tx.send(message).await {
                                log::error!("Could not send message to client: {}", error);
                            }
                        }
                        Err(error) => log::error!("Could not serialize message: {}", error),
                    }
                },
                Err(error) => {
                    log::warn!("Could not receive message(s) from arena: {}", error);
                }

            },
            /* handle requests from client */
            Some(rx) = websocket_rx.next() => match rx {
                Ok(message) => {
                    if message.is_close() {
                        break;
                    }
                    match bincode::deserialize::<UpMessage>(message.as_bytes()) {
                        Ok(message) => match message {
                            UpMessage::DroneAction(id, action) => {
                                let request = drone::Request::Execute(action);
                                let _ = arena_tx.send(arena::Request::ForwardDroneRequest(id, request)).await;
                            },
                            UpMessage::PiPuckAction(id, action) => todo!(),
                            UpMessage::Experiment(request) => {
                                let _ = arena_tx.send(arena::Request::Process(request)).await;
                            },
                        },
                        Err(_) => todo!(),
                    }
                }
                Err(error) => {
                    log::warn!("{}", error);
                }
            },
            /* stream pipuck updates to client */
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
            /* stream drone updates to client */
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
) -> anyhow::Result<StreamMap<Arc<drone::Descriptor>, BroadcastStream<drone::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Request::GetDroneDescriptors(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not request drone descriptors"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get drone descriptors")))
        .and_then(|drone_descs| drone_descs.into_iter()
            .map(|drone_desc| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let request = drone::Request::Subscribe(callback_tx);
                arena_tx.send(arena::Request::ForwardDroneRequest(drone_desc.id.clone(), request))
                    .map_err(|_| anyhow::anyhow!("Could not request drone updates"))
                    .and_then(|_| callback_rx
                        .map(|result| result.context("Could not subscribe to drone updates"))
                        .map_ok(|receiver| (drone_desc, BroadcastStream::new(receiver))))
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>()
        ).await?;
    let mut drone_update_stream_map = StreamMap::new();
    for (desc, update_stream) in update_streams {
        drone_update_stream_map.insert(desc, update_stream);
    }
    Ok(drone_update_stream_map)
}

async fn subscribe_pipuck_updates(
    arena_tx: &mpsc::Sender<arena::Request>
) -> anyhow::Result<StreamMap<Arc<pipuck::Descriptor>, BroadcastStream<pipuck::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Request::GetPiPuckDescriptors(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not request Pi-Puck descriptors"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get Pi-Puck descriptors")))
        .and_then(|pipuck_descs| pipuck_descs.into_iter()
            .map(|pipuck_desc| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let request = pipuck::Request::Subscribe(callback_tx);
                arena_tx.send(arena::Request::ForwardPiPuckRequest(pipuck_desc.id.clone(), request))
                    .map_err(|_| anyhow::anyhow!("Could not request Pi-Puck updates"))
                    .and_then(|_| callback_rx
                        .map(|result| result.context("Could not subscribe to Pi-Puck updates"))
                        .map_ok(|receiver| (pipuck_desc, BroadcastStream::new(receiver))))
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>()
        ).await?;
    
    let mut pipuck_update_stream_map = StreamMap::new();
    for (desc, update_stream) in update_streams {
        pipuck_update_stream_map.insert(desc, update_stream);
    }
    Ok(pipuck_update_stream_map)
}

