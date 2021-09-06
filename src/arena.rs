
use anyhow::{Result, Context};
use futures::{StreamExt, TryFutureExt};
use serde::{Deserialize, Serialize};
use software::Software;
use std::sync::Arc;
use std::{collections::HashMap};
use log;

use tokio::sync::{broadcast, mpsc, oneshot};
use futures::{TryStreamExt, stream::FuturesUnordered};

use crate::robot::{pipuck, drone};
use crate::software;
use crate::journal;
use crate::network::{xbee, fernbedienung};
use crate::webui;
use shared::experiment::{self, State};


pub enum Request {
    /* Drone requests */
    ForwardDroneRequest(String, drone::Request),
    GetDroneDescriptors(oneshot::Sender<Vec<Arc<drone::Descriptor>>>),
    /* Pi-Puck requests */
    ForwardPiPuckRequest(String, pipuck::Request),
    GetPiPuckDescriptors(oneshot::Sender<Vec<Arc<pipuck::Descriptor>>>),
    /* Arena requests */
    AddXbee(xbee::Device, macaddr::MacAddr6),
    AddFernbedienung(fernbedienung::Device, macaddr::MacAddr6),
    /* Experiment requests */
    Process(experiment::Request),
}

pub async fn new(
    mut arena_request_rx: mpsc::Receiver<Request>,
    journal_requests_tx: &mpsc::Sender<journal::Request>,
    webui_requests_tx: broadcast::Sender<webui::Request>,
    pipucks: Vec<pipuck::Descriptor>,
    drones: Vec<drone::Descriptor>
) {
    let mut state = State::Standby;
    
    let mut drone_software : crate::software::Software = Default::default();   
    let mut pipuck_software : crate::software::Software = Default::default();
    let pipucks: HashMap<Arc<pipuck::Descriptor>, pipuck::Instance> = pipucks
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), pipuck::Instance::default()))
        .collect();
    let drones: HashMap<Arc<drone::Descriptor>, drone::Instance> = drones
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), drone::Instance::default()))
        .collect();
    
    while let Some(request) = arena_request_rx.recv().await {
        match request {
            Request::AddXbee(device, macaddr) => {
                match &associate_xbee_device(macaddr, &drones)[..] {
                    [instance] => {
                        let request = drone::Request::AssociateXbee(device);
                        let _ = instance.request_tx.send(request).await;
                    },
                    [_, _, ..] => log::error!("Xbee {} is associated with multiple drones", macaddr),
                    [] => log::warn!("Xbee {} is not associated with any drone", macaddr),
                }
            },
            Request::AddFernbedienung(device, macaddr) => {
                /* first: attempt to associate fernbedienung with a drone */
                match &associate_fernbedienung_device_with_drone(macaddr, &drones)[..] {
                    [instance] => {
                        let request = drone::Request::AssociateFernbedienung(device);
                        let _ = instance.request_tx.send(request).await;
                    },
                    [_, _, ..] => log::error!("Fernbedienung {} is associated with multiple drones", macaddr),
                    /* second: attempt to associate fernbedienung with a Pi-Puck */
                    [] => match &associate_fernbedienung_device_with_pipuck(macaddr, &pipucks)[..] {
                        [instance] => {
                            let request = pipuck::Request::AssociateFernbedienung(device);
                            let _ = instance.request_tx.send(request).await;
                        },
                        [_, _, ..] => log::error!("Fernbedienung {} is associated with multiple Pi-Pucks", macaddr),
                        [] => log::warn!("Fernbedienung {} is not associated with any drone or Pi-Puck", macaddr),
                    }
                }
            },
            /* Arena requests */
            Request::Process(request) => match request {
                experiment::Request::StartExperiment => {
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
                experiment::Request::StopExperiment => {
                    stop_experiment(&pipucks, &drones, &journal_requests_tx).await;
                    state = State::Standby;
                }
                experiment::Request::AddDroneSoftware(path, contents) => {
                    drone_software.add(path, contents);
                    let update = experiment::Update::DroneSoftware {
                        checksums: drone_software.checksums()
                            .into_iter()
                            .map(|(name, digest)| (name, format!("{:x}", digest)))
                            .collect(),
                        status: drone_software.check_config()
                            .map_err(|e| e.to_string())
                    };
                    let down_msg = shared::DownMessage::UpdateExperiment(update);
                    let request = webui::Request::BroadcastDownMessage(down_msg);
                    let _ = webui_requests_tx.send(request);
                },
                experiment::Request::ClearDroneSoftware => {
                    drone_software.clear();
                    let update = experiment::Update::DroneSoftware {
                        checksums: drone_software.checksums()
                            .into_iter()
                            .map(|(name, digest)| (name, format!("{:x}", digest)))
                            .collect(),
                        status: drone_software.check_config()
                            .map_err(|e| e.to_string())
                    };
                    let down_msg = shared::DownMessage::UpdateExperiment(update);
                    let request = webui::Request::BroadcastDownMessage(down_msg);
                    let _ = webui_requests_tx.send(request);
                },
                _ => todo!()
            },
            Request::ForwardDroneRequest(id, request) => {
                match drones.iter().find(|&(desc, _)| desc.id == id) {
                    Some((_, instance)) => {
                        let _ = instance.request_tx.send(request).await;
                    }
                    None => log::warn!("Could not find drone with identifier {}", id),
                }
            }
            Request::GetDroneDescriptors(callback) => {
                let _ = callback.send(drones.keys().cloned().collect::<Vec<_>>());
            },
            /* Pi-Puck requests */
            Request::ForwardPiPuckRequest(id, request) => {
                match pipucks.iter().find(|&(desc, _)| desc.id == id) {
                    Some((_, instance)) => {
                        let _ = instance.request_tx.send(request).await;
                    }
                    None => log::warn!("Could not find drone with identifier {}", id),
                }
            },
            Request::GetPiPuckDescriptors(callback) => {
                let _ = callback.send(pipucks.keys().cloned().collect::<Vec<_>>());
            }
        }
    }
}

fn associate_xbee_device(
    macaddr: macaddr::MacAddr6,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
) -> Vec<&drone::Instance> {
    drones.into_iter().filter_map(|(desc, instance)| {
        if desc.xbee_macaddr == macaddr {
            Some(instance)
        }
        else {
            None
        }
    }).collect::<Vec<_>>()
}

fn associate_fernbedienung_device_with_drone(
    macaddr: macaddr::MacAddr6,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
) -> Vec<&drone::Instance> {
    drones.into_iter().filter_map(|(desc, instance)| {
        if desc.upcore_macaddr == macaddr {
            Some(instance)
        }
        else {
            None
        }
    }).collect::<Vec<_>>()
}

fn associate_fernbedienung_device_with_pipuck(
    macaddr: macaddr::MacAddr6,
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
) -> Vec<&pipuck::Instance> {
    pipucks.into_iter().filter_map(|(desc, instance)| {
        if desc.rpi_macaddr == macaddr {
            Some(instance)
        }
        else {
            None
        }
    }).collect::<Vec<_>>()
}

async fn stop_experiment(
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
    journal_requests_tx: &mpsc::Sender<journal::Request>
) {
    let _ = journal_requests_tx.send(journal::Request::Stop).await;
    let drone_requests = drones
        .values()
        .map(|instance| instance.request_tx.send(drone::Request::StopExperiment))
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>();
    let pipuck_requests = pipucks
        .values()
        .map(|instance| instance.request_tx.send(pipuck::Request::StopExperiment))
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
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
    pipuck_software: &Software,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
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
        .iter()
        .map(|(id, instance)| {
            let journal_requests_tx = journal_requests_tx.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = pipuck::Request::StartExperiment {
                software: pipuck_software.clone(),
                journal: journal_requests_tx,
                callback: response_tx
            };
            async move {
                let _ = instance.request_tx.send(request).await;
                let error_msg = ||
                    format!("Could not start experiment on {}", id);
                response_rx.await
                    .context(error_msg())
                    .and_then(|inner| inner.context(error_msg()))
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* now start the drones */
    drones
        .iter()
        .map(|(id, instance)| {
            let journal_requests_tx = journal_requests_tx.clone();
            let (response_tx, response_rx) = oneshot::channel();
            let request = drone::Request::StartExperiment {
                software: drone_software.clone(),
                journal: journal_requests_tx,
                callback: response_tx
            };
            async move {
                let _ = instance.request_tx.send(request).await;
                let error_msg = ||
                    format!("Could not start experiment on {}", id);
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
