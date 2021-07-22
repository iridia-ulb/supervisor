
use anyhow::Context;
use serde::{Deserialize, Serialize};
use software::Software;
use std::{collections::HashMap, time::Duration};
use futures::{StreamExt, TryStreamExt, stream::FuturesUnordered};
use log;
use tokio::sync::{mpsc, oneshot};

use crate::robot::{pipuck, drone};
use crate::software;
use crate::journal;
use crate::network::{xbee, fernbedienung};
use crate::webui;


#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("PiPuck {0} error: {1}")]
    PiPuckError(String, pipuck::Error),
    
    #[error("Drone {0} error: {1}")]
    DroneError(String, drone::Error),

    #[error(transparent)]
    JournalError(#[from] journal::Error),
    
    #[error(transparent)]
    SoftwareError(#[from] software::Error),

    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),
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
    Refresh,

    GetActions(oneshot::Sender<Vec<Action>>),
    Execute(Action),

    AddXbee(xbee::Device, macaddr::MacAddr6),
    AddFernbedienung(fernbedienung::Device, macaddr::MacAddr6),

    /* Drone requests */
    AddDroneSoftware(String, Vec<u8>),
    ClearDroneSoftware,
    CheckDroneSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardDroneRequest(String, drone::Request),
    GetDroneIds(oneshot::Sender<Vec<String>>),
    //ForwardDroneActionAll(drone::Action),
    ///GetDrones(oneshot::Sender<HashMap<String, drone::State>>),

    /* Pi-Puck requests */
    AddPiPuckSoftware(String, Vec<u8>),
    ClearPiPuckSoftware,
    CheckPiPuckSoftware(oneshot::Sender<(software::Checksums, software::Result<()>)>),
    ForwardPiPuckRequest(String, pipuck::Request),
    GetPiPuckIds(oneshot::Sender<Vec<String>>),
    //ForwardPiPuckActionAll(pipuck::Action),
    //GetPiPucks(oneshot::Sender<HashMap<String, pipuck::State>>),
}

// TODO the table of known robots should be passed here
pub async fn new(
    mut arena_request_rx: mpsc::Receiver<Request>,
    journal_requests_tx: &mpsc::Sender<journal::Request>,
    webui_requests_tx: &mpsc::Sender<webui::Request>,
    pipucks: Vec<pipuck::Descriptor>,
    drones: Vec<drone::Descriptor>
) {
    let mut state = State::Standby;
    
    let mut drone_software : crate::software::Software = Default::default();   
    let mut pipuck_software : crate::software::Software = Default::default();
    let pipucks: HashMap<String, pipuck::Instance> = pipucks
        .into_iter()
        .map(|descriptor| (descriptor.id.clone(), pipuck::Instance::new(descriptor)))
        .collect();
    let drones: HashMap<String, drone::Instance> = drones
        .into_iter()
        .map(|descriptor| (descriptor.id.clone(), drone::Instance::new(descriptor)))
        .collect();
    
    while let Some(request) = arena_request_rx.recv().await {
        match request {
            Request::Refresh => {
                todo!("Request all devices to sync there state");
            }
            Request::AddXbee(device, macaddr) => {
                let instance = drones
                    .values()
                    .find(|instance| instance.descriptor.xbee_macaddr == macaddr);
                if let Some(instance) = instance {
                    let request = drone::Request::AssociateXbee(device);
                    let _ = instance.request_tx.send(request).await;
                }
                else {
                    log::warn!("Xbee {} detected but not specified in the configuration file", macaddr)
                }
            },
            Request::AddFernbedienung(device, macaddr) => {
                /* first: attempt to associate it with a drone */
                let instance = drones
                    .values()
                    .find(|instance| instance.descriptor.upcore_macaddr == macaddr);
                if let Some(instance) = instance {
                    let request = drone::Request::AssociateFernbedienung(device);
                    let _ = instance.request_tx.send(request).await;
                }
                else {
                    /* second: attempt to associate it with a pipuck */
                    let instance = pipucks
                        .values()
                        .find(|instance| instance.descriptor.rpi_macaddr == macaddr);
                    if let Some(instance) = instance {
                        let request = pipuck::Request::AssociateFernbedienung(device);
                        let _ = instance.request_tx.send(request).await;
                    }
                    else {
                        log::warn!("Fernbedienung {} detected but not specified in the configuration file", macaddr)
                    }
                }
            },
            /* Arena requests */
            Request::GetActions(callback) => {
                let _ = callback.send(match state {
                    State::Standby => vec![Action::StartExperiment],
                    State::Active => vec![Action::StopExperiment],
                });                
            },
            Request::Execute(action) => match action {
                Action::StartExperiment => {
                    let start_experiment_result = 
                        start_experiment(&pipucks,
                                         &pipuck_software,
                                         &drones,
                                         &drone_software,
                                         &journal_requests_tx).await;
                    match start_experiment_result {
                        Ok(_) => state = State::Active,
                        Err(error) => {
                            stop_experiment(&pipucks, &drones, &journal_requests_tx).await;
                            log::error!("Could not start experiment: {}", error);
                        }
                    };
                },
                Action::StopExperiment => {
                    stop_experiment(&pipucks, &drones, &journal_requests_tx).await;
                    state = State::Standby;
                }
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
            Request::ForwardDroneRequest(id, request) => match drones.get(&id) {
                Some(instance) => {
                    let _ = instance.request_tx.send(request).await;
                }
                None => log::warn!("Could not find {}", id),
            }
            Request::GetDroneIds(callback) => {
                callback.send(drones.keys().map(String::to_owned).collect::<Vec<_>>());
            }
            /* Pi-Puck requests */
            Request::AddPiPuckSoftware(path, contents) =>
                pipuck_software.add(path, contents),
            Request::ClearPiPuckSoftware =>
                pipuck_software.clear(),
            Request::CheckPiPuckSoftware(callback) => {
                let _ = callback
                    .send((pipuck_software.checksums(), pipuck_software.check_config()));
            },
            Request::ForwardPiPuckRequest(id, request) => match pipucks.get(&id) {
                Some(instance) => {
                    let _ = instance.request_tx.send(request).await;
                }
                None => log::warn!("Could not find {}", id),
            },
            Request::GetPiPuckIds(callback) => {
                callback.send(pipucks.keys().map(String::to_owned).collect::<Vec<_>>());
            }
        }
    }
}

async fn stop_experiment(
    pipucks: &HashMap<String, pipuck::Instance>,
    drones: &HashMap<String, drone::Instance>,
    journal_requests_tx: &mpsc::Sender<journal::Request>
) {
    let _ = journal_requests_tx.send(journal::Request::Stop).await;
    let drone_requests = drones
        .values()
        .map(|instance| instance.request_tx.send(drone::Request::ExperimentStop))
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>();
    let pipuck_requests = pipucks
        .values()
        .map(|instance| instance.request_tx.send(pipuck::Request::ExperimentStop))
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>();
    let (drone_result, pipuck_result) = tokio::join!(drone_requests, pipuck_requests);
    if let Err(_) = drone_result {
        log::error!("Could not stop all drones");
    }
    if let Err(_) = pipuck_result {
        log::error!("Could not stop all Pi-Pucks");
    }
}

async fn start_experiment(
    pipucks: &HashMap<String, pipuck::Instance>,
    pipuck_software: &Software,
    drones: &HashMap<String, drone::Instance>,
    drone_software: &Software,
    journal_requests_tx: &mpsc::Sender<journal::Request>
) -> anyhow::Result<()> {
    /* check software validity before starting */
    if pipucks.len() > 0 {
        pipuck_software.check_config()?;
    }
    if drones.len() > 0 {
        drone_software.check_config()?;
    }   
    /* start an experiment journal to record events during the experiment */
    let (callback_tx, callback_rx) = oneshot::channel();
    journal_requests_tx
        .send(journal::Request::Start(callback_tx)).await
        .map_err(|_| journal::Error::RequestError)?;
    callback_rx.await
        .map_err(|_| journal::Error::ResponseError)
        .and_then(|error| error)?;
    /* start the experiment */
    /* start pi-pucks first since they are less dangerous */
    pipucks
        .values()
        .map(|instance| {
            let journal_requests_tx = journal_requests_tx.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = pipuck::Request::ExperimentStart {
                software: pipuck_software.clone(),
                journal: journal_requests_tx,
                callback: response_tx
            };
            async move {
                let _ = instance.request_tx.send(request).await;
                let error_msg = ||
                    format!("Could not start experiment on {}", instance.descriptor.id);
                response_rx.await
                    .context(error_msg())
                    .and_then(|inner| inner.context(error_msg()))
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* now start the drones */
    drones
        .values()
        .map(|instance| {
            let journal_requests_tx = journal_requests_tx.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = drone::Request::ExperimentStart {
                software: drone_software.clone(),
                journal: journal_requests_tx,
                callback: response_tx
            };
            async move {
                let _ = instance.request_tx.send(request).await;
                let error_msg = ||
                    format!("Could not start experiment on {}", instance.descriptor.id);
                response_rx.await
                    .context(error_msg())
                    .and_then(|inner| inner.context(error_msg()))
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* experiment started successfully */
    Ok(())
}

// async fn handle_get_pipucks_request(pipuck_tx_map: &HashMap<Uuid, pipuck::Sender>,
//                                     callback: oneshot::Sender<HashMap<Uuid, pipuck::State>>) {
//     let pipuck_states = pipuck_tx_map
//         .into_iter()
//         .filter_map(|(uuid, tx)| {
//             let uuid = uuid.clone();
//             let (response_tx, response_rx) = oneshot::channel();
//             let request = pipuck::Request::State(response_tx);
//             tx.send(request).await.map(|_| async move {
//                 (uuid, response_rx.await)
//             }).ok()
//         })
//         .collect::<FuturesUnordered<_>>()
//         .filter_map(|(uuid, result)| async move {
//             result.ok().map(|state| (uuid, state))
//         })
//         .collect::<HashMap<_,_>>().await;
//     if let Err(_) = callback.send(pipuck_states) {
//         log::error!("Could not respond with Pi-Puck states")
//     }
// }

// async fn handle_get_drones_request(drone_tx_map: &HashMap<Uuid, drone::Sender>,
//                                    callback: oneshot::Sender<HashMap<Uuid, drone::State>>) {
//     let drone_states = drone_tx_map
//         .into_iter()
//         .filter_map(|(uuid, tx)| {
//             let uuid = uuid.clone();
//             let (response_tx, response_rx) = oneshot::channel();
//             let request = drone::Request::GetState(response_tx);
//             tx.send(request).await.map(|_| async move {
//                 (uuid, response_rx.await)
//             }).ok()
//         })
//         .collect::<FuturesUnordered<_>>()
//         .filter_map(|(uuid, result)| async move {
//             result.ok().map(|state| (uuid, state))
//         })
//         .collect::<HashMap<_,_>>().await;
//     if let Err(_) = callback.send(drone_states) {
//         log::error!("Could not respond with drone states")
//     }
// }