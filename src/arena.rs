
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /* Drone requests */
    AddDrone(Drone),
    AddDroneSoftware(String, Vec<u8>),
    ClearDroneSoftware,
    CheckDroneSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardDroneAction(Uuid, drone::Action),
    ForwardDroneActionAll(drone::Action),
    GetDrones(oneshot::Sender<HashMap<Uuid, drone::State>>),
    /* Pi-Puck requests */
    AddPiPuck(PiPuck),
    AddPiPuckSoftware(String, Vec<u8>),
    ClearPiPuckSoftware,
    CheckPiPuckSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardPiPuckAction(Uuid, pipuck::Action),
    ForwardPiPuckActionAll(pipuck::Action),
    GetPiPucks(oneshot::Sender<HashMap<Uuid, pipuck::State>>),
}

pub async fn new(arena_request_rx: mpsc::UnboundedReceiver<Request>) {
    let mut state = State::Standby;

    let mut requests = UnboundedReceiverStream::new(arena_request_rx);
    
    let mut drones : FuturesUnordered<Drone> = Default::default();
    let mut pipucks : FuturesUnordered<PiPuck> = Default::default();

    let mut drone_software : crate::software::Software = Default::default();
    let mut pipuck_software : crate::software::Software = Default::default();

    // if let Some(request) = requests.next().await {
    //     match request {
    //         _ => {},               
    //     }
    // }
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
                /* Drone requests */
                Request::AddDrone(drone) => drones.push(drone),
                Request::AddDroneSoftware(path, contents) => drone_software.0.push((path, contents)),
                Request::ClearDroneSoftware => drone_software.0.clear(),
                Request::CheckDroneSoftware(callback) => {
                    let checksums = drone_software.checksums();
                    let check = drone_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with drone software check");
                    }
                },
                Request::ForwardDroneAction(uuid, action) => 
                    handle_forward_drone_action(uuid, action, drones.iter()).await,
                Request::ForwardDroneActionAll(action) => {
                    for drone in drones.iter() {
                        let request = drone::Request::Execute(action);
                        if let Err(error) = drone.tx.send((request, None)) {
                            log::error!("Could not send action to drone {}: {}", drone.uuid, error);
                        }
                    }
                },
                Request::GetDrones(callback) => 
                    handle_get_drones_request(drones.iter(), callback).await,
                /* Pi-Puck requests */
                Request::AddPiPuck(pipuck) => pipucks.push(pipuck),
                Request::AddPiPuckSoftware(path, contents) => pipuck_software.0.push((path, contents)),
                Request::ClearPiPuckSoftware => pipuck_software.0.clear(),
                Request::CheckPiPuckSoftware(callback) => {
                    let checksums = pipuck_software.checksums();
                    let check = pipuck_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with Pi-Puck software check");
                    }
                },
                Request::ForwardPiPuckAction(uuid, action) => 
                    handle_forward_pipuck_action(uuid, action, pipucks.iter()).await,
                Request::ForwardPiPuckActionAll(action) => {
                    for pipuck in pipucks.iter() {
                        let request = pipuck::Request::Execute(action);
                        if let Err(error) = pipuck.tx.send((request, None)) {
                            log::error!("Could not send action to Pi-Puck {}: {}", pipuck.uuid, error);
                        }
                    }
                },
                Request::GetPiPucks(callback) => 
                    handle_get_pipucks_request(pipucks.iter(), callback).await,
            },
            Some(result) = drones.next() => match result {
                // todo: return uuid, ip of robot?
                // return unused ip address to pool?
                Ok(_) => log::info!("Robot task completed sucessfully"),
                Err(error) => log::error!("Robot task failed: {}", error),
            },
            Some(result) = pipucks.next() => match result {
                // todo: return uuid, ip of robot?
                // return unused ip address to pool?
                Ok(_) => log::info!("Robot task completed sucessfully"),
                Err(error) => log::error!("Robot task failed: {}", error),
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

async fn handle_forward_pipuck_action<'a, P>(uuid: Uuid, action: pipuck::Action, pipucks: P)
    where P: IntoIterator<Item = &'a PiPuck> {
    if let Some(pipuck) = pipucks.into_iter().find(|pipuck| pipuck.uuid == uuid) {
        let request = pipuck::Request::Execute(action);
        if let Err(error) = pipuck.tx.send((request, None)) {
            log::error!("Could not send action {:?} to Pi-Puck {}: {}", action, uuid, error);
        }
    }
    else {
        log::error!("Could not find Pi-Puck {}", uuid);
    }
}

async fn handle_get_pipucks_request<'a, P>(pipucks: P, callback: oneshot::Sender<HashMap<Uuid, pipuck::State>>)
    where P: IntoIterator<Item = &'a PiPuck> {
    let pipuck_states = pipucks
        .into_iter()
        .filter_map(|pipuck| {
            let (response_tx, response_rx) = oneshot::channel();
            let request = (pipuck::Request::GetState, Some(response_tx));
            pipuck.tx.send(request).map(|_| async move {
                (pipuck.uuid, response_rx.await)
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

async fn handle_forward_drone_action<'a, D>(uuid: Uuid, action: drone::Action, drones: D)
    where D: IntoIterator<Item = &'a Drone> {
    if let Some(drone) = drones.into_iter().find(|drone| drone.uuid == uuid) {
        let request = drone::Request::Execute(action);
        if let Err(error) = drone.tx.send((request, None)) {
            log::error!("Could not send action {:?} to drone {}: {}", action, uuid, error);
        }
    }
    else {
        log::error!("Could not find drone {}", uuid);
    }
}

async fn handle_get_drones_request<'a, D>(drones: D, callback: oneshot::Sender<HashMap<Uuid, drone::State>>)
    where D: IntoIterator<Item = &'a Drone> {
    let drone_states = drones
        .into_iter()
        .filter_map(|drone| {
            let (response_tx, response_rx) = oneshot::channel();
            let request = (drone::Request::GetState, Some(response_tx));
            drone.tx.send(request).map(|_| async move {
                (drone.uuid, response_rx.await)
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