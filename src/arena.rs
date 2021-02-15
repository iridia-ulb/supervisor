
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::Ipv4Addr};
use futures::{StreamExt, stream::FuturesUnordered};
use log;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{robot::{pipuck::{self, PiPuck}, drone::{self, Drone}}, software};


#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    PiPuckError(#[from] pipuck::Error),
    #[error(transparent)]
    DroneError(#[from] drone::Error),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Start Experiment")]
    StartExperiment,
    #[serde(rename = "Stop Experiment")]
    StopExperiment,
}

pub type Result<T> = std::result::Result<T, Error>;

enum State {
    Standby,
    Active,
}

pub enum Request {
    /* Arena requests */
    GetActions(oneshot::Sender<Vec<Action>>),
    Execute(Action),
    /* Drone requests */
    AddDrone(Uuid, drone::Sender, Drone),
    AddDroneSoftware(String, Vec<u8>),
    ClearDroneSoftware,
    CheckDroneSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardDroneAction(Uuid, drone::Action),
    ForwardDroneActionAll(drone::Action),
    GetDrones(oneshot::Sender<HashMap<Uuid, drone::State>>),
    /* Pi-Puck requests */
    AddPiPuck(Uuid, pipuck::Sender, PiPuck),
    AddPiPuckSoftware(String, Vec<u8>),
    ClearPiPuckSoftware,
    CheckPiPuckSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardPiPuckAction(Uuid, pipuck::Action),
    ForwardPiPuckActionAll(pipuck::Action),
    GetPiPucks(oneshot::Sender<HashMap<Uuid, pipuck::State>>),
}

pub async fn new(arena_request_rx: mpsc::UnboundedReceiver<Request>,
                 network_addr_tx: mpsc::UnboundedSender<Ipv4Addr>) {
    let mut state = State::Standby;

    let mut requests = UnboundedReceiverStream::new(arena_request_rx);
    
    let mut drone_software : crate::software::Software = Default::default();
    let mut drone_tasks : FuturesUnordered<Drone> = Default::default();
    let mut drone_tx_map : HashMap<Uuid, drone::Sender> = Default::default();
    
    let mut pipuck_software : crate::software::Software = Default::default();
    let mut pipuck_tasks : FuturesUnordered<PiPuck> = Default::default();
    let mut pipuck_tx_map : HashMap<Uuid, pipuck::Sender> = Default::default();

    loop {
        tokio::select! {
            Some(request) = requests.next() => match request {
                /* Arena requests */
                Request::GetActions(callback) => {
                    let actions = match state {
                        State::Standby => vec![Action::StartExperiment],
                        State::Active => vec![Action::StopExperiment],
                    };
                    if let Err(_) = callback.send(actions) {
                        log::error!("Could not respond with arena actions");
                    }
                },
                Request::Execute(action) => {
                    log::info!("{:?}", action);
                },
                /* Drone requests */
                Request::AddDrone(uuid, tx, task) => {
                    drone_tx_map.insert(uuid, tx);
                    drone_tasks.push(task)
                }
                Request::AddDroneSoftware(path, contents) => drone_software.add(path, contents),
                Request::ClearDroneSoftware => drone_software.clear(),
                Request::CheckDroneSoftware(callback) => {
                    let checksums = drone_software.checksums();
                    let check = drone_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with drone software check");
                    }
                },
                Request::ForwardDroneAction(uuid, action) => 
                    handle_forward_drone_action(uuid, action, &drone_tx_map),
                Request::ForwardDroneActionAll(action) => {
                    for (uuid, tx) in drone_tx_map.iter() {
                        let request = drone::Request::Execute(action);
                        if let Err(error) = tx.send((request, None)) {
                            log::error!("Could not send action to drone {}: {}", uuid, error);
                        }
                    }
                },
                Request::GetDrones(callback) => 
                    handle_get_drones_request(&drone_tx_map, callback).await,
                /* Pi-Puck requests */
                Request::AddPiPuck(uuid, tx, task) => {
                    pipuck_tx_map.insert(uuid, tx);
                    pipuck_tasks.push(task)
                },
                Request::AddPiPuckSoftware(path, contents) => pipuck_software.add(path, contents),
                Request::ClearPiPuckSoftware => pipuck_software.clear(),
                Request::CheckPiPuckSoftware(callback) => {
                    let checksums = pipuck_software.checksums();
                    let check = pipuck_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with Pi-Puck software check");
                    }
                },
                Request::ForwardPiPuckAction(uuid, action) => 
                    handle_forward_pipuck_action(uuid, action, &pipuck_tx_map),
                Request::ForwardPiPuckActionAll(action) => {
                    for (uuid, tx) in pipuck_tx_map.iter() {
                        let request = pipuck::Request::Execute(action);
                        if let Err(error) = tx.send((request, None)) {
                            log::error!("Could not send action to Pi-Puck {}: {}", uuid, error);
                        }
                    }
                },
                Request::GetPiPucks(callback) => 
                    handle_get_pipucks_request(&pipuck_tx_map, callback).await,
            },
            Some(result) = drone_tasks.next() => match result {
                Ok((uuid, xbee_addr, linux_addr)) => {
                    drone_tx_map.remove(&uuid);
                    if let Err(error) = network_addr_tx.send(xbee_addr) {
                        log::error!("Could not return the Xbee address of drone {} to the network module: {}", uuid, error);
                    }
                    if let Some(linux_addr) = linux_addr {
                        if let Err(error) = network_addr_tx.send(linux_addr) {
                            log::error!("Could not return the Linux address of drone {} to the network module: {}", uuid, error);
                        }
                    }
                },
                Err(error) => log::error!("Drone task panicked: {}", error),
            },
            Some(result) = pipuck_tasks.next() => match result {
                Ok((uuid, linux_addr)) => {
                    pipuck_tx_map.remove(&uuid);
                    if let Err(error) = network_addr_tx.send(linux_addr) {
                        log::error!("Could not return the Linux address of Pi-Puck {} to the network module: {}", uuid, error);
                    }
                },
                Err(error) => log::error!("Pi-Puck task panicked: {}", error),
            },
            else => {
                // I believe this only occurs when all robot tasks are complete
                // and all instances of arena_request_tx have been dropped
                log::warn!("todo! shutdown all robots? Nope, not here until robots have already shutdown");
                break;
            }
        }
    }
    log::info!("arena task is complete");
}

fn handle_forward_pipuck_action(uuid: Uuid, action: pipuck::Action, pipuck_tx_map: &HashMap<Uuid, pipuck::Sender>) {
    match pipuck_tx_map.get(&uuid) {
        Some(tx) => {
            let request = pipuck::Request::Execute(action);
            if let Err(error) = tx.send((request, None)) {
                log::warn!("Could not send action {:?} to Pi-Puck {}: {}", action, uuid, error);
            }
        }
        None => log::warn!("Could not find Pi-Puck {}", uuid)
    }
}

async fn handle_get_pipucks_request(pipuck_tx_map: &HashMap<Uuid, pipuck::Sender>,
                                    callback: oneshot::Sender<HashMap<Uuid, pipuck::State>>) {
    let pipuck_states = pipuck_tx_map
        .into_iter()
        .filter_map(|(uuid, tx)| {
            let uuid = uuid.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = (pipuck::Request::State, Some(response_tx));
            tx.send(request).map(|_| async move {
                (uuid, response_rx.await)
            }).ok()
        })
        .collect::<FuturesUnordered<_>>()
        .filter_map(|(uuid, result)| async move {
            result.ok().and_then(|response| match response {
                pipuck::Response::State(state) => Some((uuid, state)),
                _ => None
            })
        })
        .collect::<HashMap<_,_>>().await;
    if let Err(_) = callback.send(pipuck_states) {
        log::error!("Could not respond with Pi-Puck states")
    }
}

fn handle_forward_drone_action(uuid: Uuid, action: drone::Action, drone_tx_map: &HashMap<Uuid, drone::Sender>) {
    match drone_tx_map.get(&uuid) {
        Some(tx) => {
            let request = drone::Request::Execute(action);
            if let Err(error) = tx.send((request, None)) {
                log::warn!("Could not send action {:?} to drone {}: {}", action, uuid, error);
            }
        }
        None => log::warn!("Could not find drone {}", uuid)
    }
}

async fn handle_get_drones_request(drone_tx_map: &HashMap<Uuid, drone::Sender>,
                                   callback: oneshot::Sender<HashMap<Uuid, drone::State>>) {
    let drone_states = drone_tx_map
        .into_iter()
        .filter_map(|(uuid, tx)| {
            let uuid = uuid.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = (drone::Request::GetState, Some(response_tx));
            tx.send(request).map(|_| async move {
                (uuid, response_rx.await)
            }).ok()
        })
        .collect::<FuturesUnordered<_>>()
        .filter_map(|(uuid, result)| async move {
            result.ok().and_then(|response| match response {
                drone::Response::State(state) => Some((uuid, state)),
                _ => None
            })
        })
        .collect::<HashMap<_,_>>().await;
    if let Err(_) = callback.send(drone_states) {
        log::error!("Could not respond with drone states")
    }
}