use anyhow::Context;
use futures::{FutureExt, SinkExt, StreamExt, TryFutureExt, TryStreamExt, stream::{self, FuturesUnordered}};
use shared::{BackEndRequest, DownMessage, FrontEndRequest, UpMessage};
use std::{net::SocketAddr, ops::Deref, sync::Arc};
use tokio::{self, sync::{broadcast, mpsc, oneshot}};
use tokio_stream::{StreamMap, wrappers::{BroadcastStream, errors::BroadcastStreamRecvError}};
use warp::Filter;
use uuid::Uuid;

use crate::{arena, robot::{self, drone}, robot::pipuck};

// down message (from backend to the client)
// up message (from client to the backend)

/* embed the client js and wasm into this binary */
const CLIENT_WASM_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_WASM"));
const CLIENT_JS_BYTES: &'static [u8] = include_bytes!(env!("CLIENT_JS"));

pub async fn run(
    server_addr: SocketAddr,
    arena_tx: mpsc::Sender<arena::Action>
) {
    /* start the server */
    let wasm_route = warp::path("client_bg.wasm")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_WASM_BYTES, "content-type", "application/wasm"));
    let js_route = warp::path("client.js")
        .and(warp::path::end())
        .map(|| warp::reply::with_header(CLIENT_JS_BYTES, "content-type", "application/javascript"));
    let arena_tx = warp::any().map(move || arena_tx.clone());
    let socket_route = warp::path("socket")
        .and(warp::path::end())
        .and(warp::ws())
        .and(arena_tx)
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
    arena_tx: mpsc::Sender<arena::Action>,
) {
    /* subscribe to drone updates and map them to websocket messages */
    let drone_updates = match subscribe_drone_updates(&arena_tx).await {
        Ok(updates) => {
            let add_drone_messages = updates.keys()
                .cloned()
                .map(|desc| DownMessage::Request(Uuid::new_v4(), FrontEndRequest::AddDrone(desc.deref().clone())))
                .collect::<Vec<_>>();
            let update_drone_messages = updates
                .filter_map(|(desc, update)| async move {
                    match update {
                        Ok(update) => {
                            Some(DownMessage::Request(Uuid::new_v4(), FrontEndRequest::UpdateDrone(desc.id.clone(), update)))
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
                .map(|desc| DownMessage::Request(Uuid::new_v4(), FrontEndRequest::AddPiPuck(desc.deref().clone())))
                .collect::<Vec<_>>();
            let update_pipuck_messages = updates
                .filter_map(|(desc, update)| async move {
                    match update {
                        Ok(update) => {
                            Some(DownMessage::Request(Uuid::new_v4(), FrontEndRequest::UpdatePiPuck(desc.id.clone(), update)))
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
            /* handle requests from client */
            Some(rx) = websocket_rx.next() => match rx {
                Ok(message) => {
                    if message.is_close() {
                        break;
                    }
                    match bincode::deserialize::<UpMessage>(message.as_bytes()) {
                        Ok(message) => match message {
                            UpMessage::Request(uuid, request) => {
                                let result = match request {
                                    BackEndRequest::DroneRequest(id, request) => 
                                        handle_drone_request(&arena_tx, id, request).await,
                                    BackEndRequest::PiPuckRequest(id, request) =>  
                                        handle_pipuck_request(&arena_tx, id, request).await,
                                    BackEndRequest::ExperimentRequest(request) => 
                                        handle_experiment_request(&arena_tx, request).await,
                                };
                                let response = DownMessage::Response(uuid, result.map_err(|e| e.to_string()));
                                match bincode::serialize(&response) {
                                    Ok(encoded) => {
                                        let message = warp::ws::Message::binary(encoded);
                                        if let Err(error) = websocket_tx.send(message).await {
                                            log::error!("Could not send response to client: {}", error);
                                        }
                                    }
                                    Err(error) => log::error!("Could not serialize response: {}", error),
                                }
                            },
                            UpMessage::Response(uuid, result) => if let Err(error) = result {
                                log::error!("Request {} failed: {}", uuid, error);
                            }
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
    arena_tx: &mpsc::Sender<arena::Action>
) -> anyhow::Result<StreamMap<Arc<drone::Descriptor>, BroadcastStream<drone::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Action::GetDroneDescriptors(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not communicate with drone"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get drone descriptors")))
        .and_then(|drone_descs| drone_descs.into_iter()
            .map(|drone_desc| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let action = drone::Action::Subscribe(callback_tx);
                arena_tx.send(arena::Action::ForwardDroneAction(drone_desc.id.clone(), action))
                    .map_err(|_| anyhow::anyhow!("Could not communicate with drone"))
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
    arena_tx: &mpsc::Sender<arena::Action>
) -> anyhow::Result<StreamMap<Arc<pipuck::Descriptor>, BroadcastStream<pipuck::Update>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let update_streams = arena_tx.send(arena::Action::GetPiPuckDescriptors(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not communicate with Pi-Puck"))
        .and_then(|_| callback_rx
            .map(|result| result.context("Could not get Pi-Puck descriptors")))
        .and_then(|pipuck_descs| pipuck_descs.into_iter()
            .map(|pipuck_desc| {
                let (callback_tx, callback_rx) = oneshot::channel();
                let action = pipuck::Action::Subscribe(callback_tx);
                arena_tx.send(arena::Action::ForwardPiPuckAction(pipuck_desc.id.clone(), action))
                    .map_err(|_| anyhow::anyhow!("Could not communicate with Pi-Puck"))
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

async fn handle_drone_request(
    arena_tx: &mpsc::Sender<arena::Action>,
    id: String,
    request: shared::drone::Request
) -> anyhow::Result<()> {
    use shared::drone::Request;
    use robot::{FernbedienungAction, TerminalAction, XbeeAction};
    use drone::Action;
    let (callback_tx, callback_rx) = oneshot::channel();
    let action = match request {
        Request::BashTerminalStart => 
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::Bash(TerminalAction::Start)),
        Request::BashTerminalStop => 
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::Bash(TerminalAction::Stop)),
        Request::BashTerminalRun(command) => 
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::Bash(TerminalAction::Run(command))),
        Request::CameraStreamEnable(on) => 
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::SetCameraStream(on)),
        Request::PixhawkPowerEnable(on) => 
            Action::ExecuteXbeeAction(callback_tx, XbeeAction::SetPixhawkPower(on)),
        Request::MavlinkTerminalStart => 
            Action::ExecuteXbeeAction(callback_tx, XbeeAction::Mavlink(TerminalAction::Start)),
        Request::MavlinkTerminalStop => 
            Action::ExecuteXbeeAction(callback_tx, XbeeAction::Mavlink(TerminalAction::Stop)),
        Request::MavlinkTerminalRun(command) => 
            Action::ExecuteXbeeAction(callback_tx, XbeeAction::Mavlink(TerminalAction::Run(command))),
        Request::UpCorePowerEnable(on) => 
            Action::ExecuteXbeeAction(callback_tx, XbeeAction::SetUpCorePower(on)),
        Request::UpCoreHalt => 
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::Halt),
        Request::UpCoreReboot =>
            Action::ExecuteFernbedienungAction(callback_tx, FernbedienungAction::Reboot),
    };
    arena_tx.send(arena::Action::ForwardDroneAction(id, action)).await
        .map_err(|_| anyhow::anyhow!("Could not send action to arena"))?;
    callback_rx.await.map_err(|_| anyhow::anyhow!("No response from arena"))?
}

async fn handle_pipuck_request(
    _arena_tx: &mpsc::Sender<arena::Action>,
    _id: String,
    _request: shared::pipuck::Request,
) -> anyhow::Result<()> {
    Ok(())
}

async fn handle_experiment_request(
    arena_tx: &mpsc::Sender<arena::Action>,
    request: shared::experiment::Request,
) -> anyhow::Result<()> {
    use shared::experiment::Request;
    use arena::Action;
    let (callback_tx, callback_rx) = oneshot::channel();
    let action = match request {
        Request::Start { drone_software, pipuck_software } => 
            Action::StartExperiment { callback: callback_tx, drone_software, pipuck_software },
        Request::Stop =>
            Action::StopExperiment { callback: callback_tx },
    };
    arena_tx.send(action).await
        .map_err(|_| anyhow::anyhow!("Could not send action to arena"))?;
    callback_rx.await.map_err(|_| anyhow::anyhow!("No response from arena"))?
}