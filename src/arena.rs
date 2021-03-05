
use serde::{Deserialize, Serialize};
use software::Software;
use std::{collections::HashMap, net::{Ipv4Addr, SocketAddr}};
use futures::{StreamExt, TryStreamExt, stream::FuturesUnordered};
use log;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::robot::{pipuck::{self, PiPuck}, drone::{self, Drone}};
use crate::software;
use crate::journal;
use crate::network;


#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("PiPuck {0} error: {1}")]
    PiPuckError(Uuid, pipuck::Error),
    
    #[error("Drone {0} error: {1}")]
    DroneError(Uuid, drone::Error),

    #[error(transparent)]
    JournalError(#[from] journal::Error),
    
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
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
    AddDrone(network::xbee::Device),
    AddDroneSoftware(String, Vec<u8>),
    ClearDroneSoftware,
    CheckDroneSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardDroneAction(Uuid, drone::Action),
    PairWithDrone(network::fernbedienung::Device),
    //ForwardDroneActionAll(drone::Action),
    GetDrones(oneshot::Sender<HashMap<Uuid, drone::State>>),
    /* Pi-Puck requests */
    AddPiPuck(network::fernbedienung::Device),
    AddPiPuckSoftware(String, Vec<u8>),
    ClearPiPuckSoftware,
    CheckPiPuckSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardPiPuckAction(Uuid, pipuck::Action),
    //ForwardPiPuckActionAll(pipuck::Action),
    GetPiPucks(oneshot::Sender<HashMap<Uuid, pipuck::State>>),
}

pub async fn new(message_router_addr: SocketAddr,
                 arena_request_rx: mpsc::UnboundedReceiver<Request>,
                 network_addr_tx: &mpsc::UnboundedSender<Ipv4Addr>,
                 journal_requests_tx: &mpsc::UnboundedSender<journal::Request>) {
    let mut state = State::Standby;

    let mut requests = UnboundedReceiverStream::new(arena_request_rx);
    
    // does my inception of an active object allow me to put this together?
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
                Request::Execute(action) => match action {
                    Action::StartExperiment => {
                        let start_experiment_result = 
                            start_experiment(&pipuck_tx_map,
                                             &pipuck_software,
                                             &drone_tx_map,
                                             &drone_software,
                                             &message_router_addr,
                                             &journal_requests_tx).await;
                        match start_experiment_result {
                            Ok(_) => state = State::Active,
                            Err(error) => log::error!("Could not start experiment: {}", error),
                        };
                    },
                    Action::StopExperiment => {
                        if let Err(error) = stop_experiment(&journal_requests_tx).await {
                            log::warn!("Could not stop experiment: {}", error);
                        }
                        state = State::Standby;
                    }
                }
                /* Drone requests */
                Request::AddDrone(device) => {
                    let (uuid, tx, task) = Drone::new(device);
                    drone_tx_map.insert(uuid, tx);
                    drone_tasks.push(task)
                }
                Request::AddDroneSoftware(path, contents) =>
                    drone_software.add(path, contents),
                Request::ClearDroneSoftware =>
                    drone_software.clear(),
                Request::CheckDroneSoftware(callback) => {
                    let checksums = drone_software.checksums();
                    let check = drone_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with drone software check");
                    }
                },
                Request::ForwardDroneAction(uuid, action) => 
                    handle_forward_drone_action_request(&drone_tx_map, uuid, action).await,
                /*
                Request::ForwardDroneActionAll(action) => {
                    for (uuid, tx) in drone_tx_map.iter() {
                        let request = drone::Request::Execute(action);
                        if let Err(error) = tx.send((request, None)) {
                            log::error!("Could not send action to drone {}: {}", uuid, error);
                        }
                    }
                },
                */
                Request::GetDrones(callback) => 
                    handle_get_drones_request(&drone_tx_map, callback).await,
                Request::PairWithDrone(device) =>
                    handle_pair_with_drone_request(&drone_tx_map, device).await,
                /* Pi-Puck requests */
                Request::AddPiPuck(device) => {
                    let (uuid, tx, task) = PiPuck::new(device);
                    pipuck_tx_map.insert(uuid, tx);
                    pipuck_tasks.push(task)
                },
                Request::AddPiPuckSoftware(path, contents) =>
                    pipuck_software.add(path, contents),
                Request::ClearPiPuckSoftware =>
                    pipuck_software.clear(),
                Request::CheckPiPuckSoftware(callback) => {
                    let checksums = pipuck_software.checksums();
                    let check = pipuck_software.check_config();
                    if let Err(_) = callback.send((checksums, check)) {
                        log::error!("Could not respond with Pi-Puck software check");
                    }
                },
                Request::ForwardPiPuckAction(uuid, action) => 
                    handle_forward_pipuck_action_request(&pipuck_tx_map, uuid, action),
                /*
                Request::ForwardPiPuckActionAll(action) => {
                    for (uuid, tx) in pipuck_tx_map.iter() {
                        let request = pipuck::Request::Execute(action);
                        if let Err(error) = tx.send((request, None)) {
                            log::error!("Could not send action to Pi-Puck {}: {}", uuid, error);
                        }
                    }
                },
                */
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

async fn handle_pair_with_drone_request(drone_tx_map: &HashMap<Uuid, drone::Sender>,
                                        device: network::fernbedienung::Device) {
    
    // 1. write an id to the GPIO expander
    let task = network::fernbedienung::Run {
        target: "echo".into(),
        working_dir: "/tmp".into(),
        args: vec!["".to_owned()],
    };
    //device.run(task, None, None, stdout_tx, None)
        // 2. query all drones to see if they have a matching id
    // let drone_ids = drone_tx_map
    //     .into_iter()
    //     .filter_map(|(uuid, tx)| {
    //         let uuid = uuid.clone();
    //         let (response_tx, response_rx) = oneshot::channel();
    //         let request = drone::Request::GetId(response_tx);
    //         tx.send(request).map(|_| async move {
    //             (uuid, response_rx.await)
    //         }).ok()
    //     })
    //     .collect::<FuturesUnordered<_>>()
    //     .filter_map(|(uuid, result)| async move {
    //         result.ok().map(|state| (uuid, state))
    //     })
    //     .collect::<Vec<_>>().await;
    

    // 3. set the 
}


async fn stop_experiment(journal_requests_tx: &mpsc::UnboundedSender<journal::Request>) -> Result<()> {
    journal_requests_tx
        .send(journal::Request::Stop)
        .map_err(|_| journal::Error::RequestError)?;
    Ok(())
}

async fn start_experiment(pipuck_tx_map: &HashMap<Uuid, pipuck::Sender>,
                          pipuck_software: &Software,
                          drone_tx_map: &HashMap<Uuid, drone::Sender>,
                          drone_software: &Software,
                          message_router_addr: &SocketAddr,
                          journal_requests_tx: &mpsc::UnboundedSender<journal::Request>) -> Result<()> {
    if let Err(error) = pipuck_software.check_config() {
        if pipuck_tx_map.len() > 0 {
            return Err(Error::SoftwareError(error));
        }
    }
    if let Err(error) = drone_software.check_config() {
        if drone_tx_map.len() > 0 {
            return Err(Error::SoftwareError(error));
        }
    }
    /* upload software to Pi-Pucks */
    pipuck_tx_map.into_iter()
        .map(|(uuid, tx)| {
            let uuid = uuid.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let software = pipuck_software.clone();
            let request = pipuck::Request::Upload(software, response_tx);
            tx.send(request)
                .map_err(|_| Error::PiPuckError(uuid, pipuck::Error::RequestError))
                .map(|_| async move {
                    (uuid, response_rx.await)
                })
        })
        .collect::<Result<FuturesUnordered<_>>>()?
        .map(|(uuid, result)| result
            .map_err(|_| Error::PiPuckError(uuid, pipuck::Error::ResponseError))
            .and_then(|response| {
                response.map_err(|error| Error::PiPuckError(uuid, error))
            })
        ).try_collect::<Vec<_>>().await?;
    let (callback_tx, callback_rx) = oneshot::channel();
    journal_requests_tx
        .send(journal::Request::Start(callback_tx))
        .map_err(|_| journal::Error::RequestError)?;
    callback_rx.await
        .map_err(|_| journal::Error::ResponseError)
        .and_then(|error| error)?; // flatten available in nightly

    /* if an error occurs from this point onwards, there is not much that can be done
       from this function. Experiment started should be set if any robots are running */
    
    let pipuck_start = pipuck_tx_map.into_iter()
        .map(|(uuid, tx)| {
            let uuid = uuid.clone();
            let message_router_addr = message_router_addr.clone();
            let journal_requests_tx = journal_requests_tx.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = pipuck::Request::ExperimentStart(
                message_router_addr, journal_requests_tx, response_tx);
            tx.send(request)
                .map_err(|_| Error::PiPuckError(uuid, pipuck::Error::RequestError))
                .map(|_| async move {
                    (uuid, response_rx.await)
                })
        })
        .collect::<Result<FuturesUnordered<_>>>()?
        .map(|(uuid, result)| result
            .map_err(|_| Error::PiPuckError(uuid, pipuck::Error::ResponseError))
            .and_then(|response| {
                response.map_err(|error| Error::PiPuckError(uuid, error))
            })
        ).try_collect::<Vec<_>>().await;
    if let Err(error) = pipuck_start {
        log::error!("Failed to start experiment: {}", error);
    }

    Ok(())
}


fn handle_forward_pipuck_action_request(pipuck_tx_map: &HashMap<Uuid, pipuck::Sender>,
                                        uuid: Uuid,
                                        action: pipuck::Action) {
    match pipuck_tx_map.get(&uuid) {
        Some(tx) => {
            let request = pipuck::Request::Execute(action);
            if let Err(error) = tx.send(request) {
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
            let request = pipuck::Request::State(response_tx);
            tx.send(request).map(|_| async move {
                (uuid, response_rx.await)
            }).ok()
        })
        .collect::<FuturesUnordered<_>>()
        .filter_map(|(uuid, result)| async move {
            result.ok().map(|state| (uuid, state))
        })
        .collect::<HashMap<_,_>>().await;
    if let Err(_) = callback.send(pipuck_states) {
        log::error!("Could not respond with Pi-Puck states")
    }
}

async fn handle_forward_drone_action_request(drone_tx_map: &HashMap<Uuid, drone::Sender>,
                                             uuid: Uuid,
                                             action: drone::Action) {
    match drone_tx_map.get(&uuid) {
        Some(tx) => {
            let request = drone::Request::Execute(action);
            if let Err(error) = tx.send(request) {
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
            let request = drone::Request::GetState(response_tx);
            tx.send(request).map(|_| async move {
                (uuid, response_rx.await)
            }).ok()
        })
        .collect::<FuturesUnordered<_>>()
        .filter_map(|(uuid, result)| async move {
            result.ok().map(|state| (uuid, state))
        })
        .collect::<HashMap<_,_>>().await;
    if let Err(_) = callback.send(drone_states) {
        log::error!("Could not respond with drone states")
    }
}