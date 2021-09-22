
use anyhow::Context;
use futures::{StreamExt, TryStreamExt, stream::FuturesUnordered};
use log;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

use crate::robot::{pipuck, drone};
use crate::journal;
use crate::network::{xbee, fernbedienung};
use shared::experiment::software::Software;

pub enum Action {
    /* Drone actions */
    ForwardDroneAction(String, drone::Action),
    GetDroneDescriptors(oneshot::Sender<Vec<Arc<drone::Descriptor>>>),
    /* Pi-Puck actions */
    ForwardPiPuckAction(String, pipuck::Action),
    GetPiPuckDescriptors(oneshot::Sender<Vec<Arc<pipuck::Descriptor>>>),
    /* Arena actions */
    AddXbee(xbee::Device, macaddr::MacAddr6),
    AddFernbedienung(fernbedienung::Device, macaddr::MacAddr6),
    /* Experiment actions */
    StartExperiment {
        callback: oneshot::Sender<anyhow::Result<()>>,
        drone_software: Software,
        pipuck_software: Software,
    },
    StopExperiment {
        callback: oneshot::Sender<anyhow::Result<()>>,
    },
}

pub async fn new(
    mut arena_action_rx: mpsc::Receiver<Action>,
    journal_action_tx: mpsc::Sender<journal::Action>,
    pipucks: Vec<pipuck::Descriptor>,
    drones: Vec<drone::Descriptor>
) {
    let pipucks: HashMap<Arc<pipuck::Descriptor>, pipuck::Instance> = pipucks
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), pipuck::Instance::default()))
        .collect();
    let drones: HashMap<Arc<drone::Descriptor>, drone::Instance> = drones
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), drone::Instance::default()))
        .collect();
    while let Some(action) = arena_action_rx.recv().await {
        match action {
            Action::AddXbee(device, macaddr) => {
                match &associate_xbee_device(macaddr, &drones)[..] {
                    [instance] => {
                        let request = drone::Action::AssociateXbee(device);
                        let _ = instance.action_tx.send(request).await;
                    },
                    [_, _, ..] => log::error!("Xbee {} is associated with multiple drones", macaddr),
                    [] => log::warn!("Xbee {} is not associated with any drone", macaddr),
                }
            },
            Action::AddFernbedienung(device, macaddr) => {
                /* first: attempt to associate fernbedienung with a drone */
                match &associate_fernbedienung_device_with_drone(macaddr, &drones)[..] {
                    [instance] => {
                        let request = drone::Action::AssociateFernbedienung(device);
                        let _ = instance.action_tx.send(request).await;
                    },
                    [_, _, ..] => log::error!("Fernbedienung {} is associated with multiple drones", macaddr),
                    /* second: attempt to associate fernbedienung with a Pi-Puck */
                    [] => match &associate_fernbedienung_device_with_pipuck(macaddr, &pipucks)[..] {
                        [instance] => {
                            let request = pipuck::Action::AssociateFernbedienung(device);
                            let _ = instance.action_tx.send(request).await;
                        },
                        [_, _, ..] => log::error!("Fernbedienung {} is associated with multiple Pi-Pucks", macaddr),
                        [] => log::warn!("Fernbedienung {} is not associated with any drone or Pi-Puck", macaddr),
                    }
                }
            },
            /* Arena requests */
            Action::StartExperiment { callback, pipuck_software, drone_software } => {
                let start_result = start_experiment(
                    &pipucks,
                    &pipuck_software,
                    &drones,
                    &drone_software,
                    &journal_action_tx).await;
                let result = match start_result {
                    Ok(_) => Ok(()),
                    Err(start_error) => match stop_experiment(&pipucks, &drones, &journal_action_tx).await {
                        Ok(_) => Err(start_error),
                        Err(stop_error) => Err(stop_error).context(start_error),
                    }
                };
                let _ = callback.send(result);
            },
            Action::StopExperiment { callback } => {
                let result = stop_experiment(&pipucks, &drones, &journal_action_tx).await;
                let _ = callback.send(result.context("Could not stop experiment"));
            },
            Action::ForwardDroneAction(id, request) => {
                match drones.iter().find(|&(desc, _)| desc.id == id) {
                    Some((_, instance)) => {
                        let _ = instance.action_tx.send(request).await;
                    }
                    None => log::warn!("Could not find drone with identifier {}", id),
                }
            }
            Action::GetDroneDescriptors(callback) => {
                let _ = callback.send(drones.keys().cloned().collect::<Vec<_>>());
            },
            /* Pi-Puck requests */
            Action::ForwardPiPuckAction(id, request) => {
                match pipucks.iter().find(|&(desc, _)| desc.id == id) {
                    Some((_, instance)) => {
                        let _ = instance.action_tx.send(request).await;
                    }
                    None => log::warn!("Could not find drone with identifier {}", id),
                }
            },
            Action::GetPiPuckDescriptors(callback) => {
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
    journal_action_tx: &mpsc::Sender<journal::Action>
) -> anyhow::Result<()> {
    let _ = journal_action_tx.send(journal::Action::Stop).await;
    let drone_requests = drones
        .iter()
        .map(|(desc, instance)| async move {
            (desc.id.clone(), instance.action_tx.send(drone::Action::StopExperiment).await)
        })
        .collect::<FuturesUnordered<_>>()
        // do not use try_collect, it aborts before completing all futures
        .collect::<Vec<_>>();
    let pipuck_requests = pipucks
        .iter()
        .map(|(desc, instance)| async move {
            (desc.id.clone(), instance.action_tx.send(pipuck::Action::StopExperiment).await)
        })
        .collect::<FuturesUnordered<_>>()
        // do not use try_collect, it aborts before completing all futures
        .collect::<Vec<_>>();
    let (drone_results, pipuck_results) = tokio::join!(drone_requests, pipuck_requests);
    let errors: Vec<String> = drone_results
        .into_iter()
        .filter_map(|(id, result)| match result {
            Err(_) => Some(id),
            Ok(_) => None,
        })
        .chain(pipuck_results
            .into_iter()
            .filter_map(|(id, result)| match result {
                Err(_) => Some(id),
                Ok(_) => None,
            })
        )
        .collect::<Vec<_>>();
    match errors.len() {
        0 => Ok(()),
        _ => Err(anyhow::anyhow!("Could not stop: {}", errors.join(", ")))
    }
}

async fn start_experiment(
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
    pipuck_software: &Software,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
    drone_software: &Software,
    journal_requests_tx: &mpsc::Sender<journal::Action>
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
        .send(journal::Action::Start(callback_tx)).await
        .map_err(|_| anyhow::anyhow!("Could not start journal"))?;
    callback_rx.await
        .map_err(|_| anyhow::anyhow!("No response from journal"))??;
    /* send all descriptors */
    let pipuck_descriptors = pipucks
        .keys()
        .map(|desc| pipuck::Descriptor::clone(desc))
        .collect::<Vec<_>>();
    let drone_descriptors = drones
        .keys()
        .map(|desc| drone::Descriptor::clone(desc))
        .collect::<Vec<_>>();
    let descriptor_event = journal::Event::Descriptors(pipuck_descriptors, drone_descriptors);
    journal_requests_tx.send(journal::Action::Record(descriptor_event)).await
        .map_err(|_| anyhow::anyhow!("Could not send robot descriptors to journal"))?;
    /* start the experiment */
    /* start pi-pucks first since they are less dangerous */
    
    /* set up the experiment on the drones */
    drones.iter()
        .map(|(desc, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = drone::Action::SetupExperiment(
                callback_tx, 
                desc.id.clone(),
                drone_software.clone(),
                journal_requests_tx.clone()
            );
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to drone"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from drone"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* start the pipucks */
    /* start the drones */
    drones.iter()
        .map(|(_, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = drone::Action::StartExperiment(callback_tx);
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to drone"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from drone"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    Ok(())
}
