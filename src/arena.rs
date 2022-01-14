
use anyhow::Context;
use futures::{StreamExt, TryStreamExt, stream::FuturesUnordered};
use log;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

use crate::robot::{builderbot, drone, pipuck};
use crate::journal;
use crate::network::{xbee, fernbedienung};
use shared::experiment::software::Software;

pub enum Action {
    /* BuilderBot actions */
    ForwardBuilderBotAction(String, builderbot::Action),
    GetBuilderBotDescriptors(oneshot::Sender<Vec<Arc<builderbot::Descriptor>>>),
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
        builderbot_software: Software,
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
    builderbots: Vec<builderbot::Descriptor>,
    drones: Vec<drone::Descriptor>,
    pipucks: Vec<pipuck::Descriptor>
) {
    let builderbots: HashMap<Arc<builderbot::Descriptor>, builderbot::Instance> = builderbots
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), builderbot::Instance::default()))
        .collect();
    let drones: HashMap<Arc<drone::Descriptor>, drone::Instance> = drones
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), drone::Instance::default()))
        .collect();
    let pipucks: HashMap<Arc<pipuck::Descriptor>, pipuck::Instance> = pipucks
        .into_iter()
        .map(|descriptor| (Arc::new(descriptor), pipuck::Instance::default()))
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
                        /* third: attempt to associate fernbedienung with a BuilderBot */
                        [] => match &associate_fernbedienung_device_with_builderbot(macaddr, &builderbots)[..] {
                            [instance] => {
                                let request = builderbot::Action::AssociateFernbedienung(device);
                                let _ = instance.action_tx.send(request).await;
                            },
                            [_, _, ..] => log::error!("Fernbedienung {} is associated with multiple BuilderBots", macaddr),
                            [] => log::warn!("Fernbedienung {} is not associated with any robot", macaddr),
                        },
                    }
                }
            },
            /* Arena requests */
            Action::StartExperiment { callback, builderbot_software, drone_software, pipuck_software } => {
                let start_result = start_experiment(
                    &builderbots,
                    &builderbot_software,
                    &drones,
                    &drone_software,
                    &pipucks,
                    &pipuck_software,
                    &journal_action_tx).await;
                let result = match start_result {
                    Ok(_) => Ok(()),
                    Err(start_error) => match stop_experiment(&builderbots, &drones, &pipucks, &journal_action_tx).await {
                        Ok(_) => Err(start_error),
                        Err(stop_error) => Err(stop_error).context(start_error),
                    }
                };
                let _ = callback.send(result);
            },
            Action::StopExperiment { callback } => {
                let result = stop_experiment(&builderbots, &drones, &pipucks, &journal_action_tx).await;
                let _ = callback.send(result.context("Could not stop experiment"));
            },
            Action::ForwardBuilderBotAction(id, request) => {
                match builderbots.iter().find(|&(desc, _)| desc.id == id) {
                    Some((_, instance)) => {
                        let _ = instance.action_tx.send(request).await;
                    }
                    None => log::warn!("Could not find BuilderBot with identifier {}", id),
                }
            }
            Action::GetBuilderBotDescriptors(callback) => {
                let _ = callback.send(builderbots.keys().cloned().collect::<Vec<_>>());
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

fn associate_fernbedienung_device_with_builderbot(
    macaddr: macaddr::MacAddr6,
    pipucks: &HashMap<Arc<builderbot::Descriptor>, builderbot::Instance>,
) -> Vec<&builderbot::Instance> {
    pipucks.into_iter().filter_map(|(desc, instance)| {
        if desc.duovero_macaddr == macaddr {
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
    builderbots: &HashMap<Arc<builderbot::Descriptor>, builderbot::Instance>,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
    journal_action_tx: &mpsc::Sender<journal::Action>
) -> anyhow::Result<()> {
    let _ = journal_action_tx.send(journal::Action::Stop).await;
    let builderbot_requests = builderbots
        .iter()
        .map(|(desc, instance)| async move {
            (desc.id.clone(), instance.action_tx.send(builderbot::Action::StopExperiment).await)
        })
        .collect::<FuturesUnordered<_>>()
        // do not use try_collect, it aborts before completing all futures
        .collect::<Vec<_>>();
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
    let (builderbot_results, drone_results, pipuck_results) =
        tokio::join!(builderbot_requests, drone_requests, pipuck_requests);
    let errors: Vec<String> = builderbot_results
        .into_iter()
        .filter_map(|(id, result)| match result {
            Err(_) => Some(id),
            Ok(_) => None,
        })
        .chain(drone_results
            .into_iter()
            .filter_map(|(id, result)| match result {
                Err(_) => Some(id),
                Ok(_) => None,
            })
        )
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
    builderbots: &HashMap<Arc<builderbot::Descriptor>, builderbot::Instance>,
    builderbot_software: &Software,
    drones: &HashMap<Arc<drone::Descriptor>, drone::Instance>,
    drone_software: &Software,
    pipucks: &HashMap<Arc<pipuck::Descriptor>, pipuck::Instance>,
    pipuck_software: &Software,
    journal_requests_tx: &mpsc::Sender<journal::Action>
) -> anyhow::Result<()> {
    /* check software validity before starting */
    if builderbots.len() > 0 {
        builderbot_software.check_config()?;
    }
    if drones.len() > 0 {
        drone_software.check_config()?;
    }
    if pipucks.len() > 0 {
        pipuck_software.check_config()?;
    }
    /* start an experiment journal to record events during the experiment */
    let (callback_tx, callback_rx) = oneshot::channel();
    journal_requests_tx
        .send(journal::Action::Start(callback_tx)).await
        .map_err(|_| anyhow::anyhow!("Could not start journal"))?;
    callback_rx.await
        .map_err(|_| anyhow::anyhow!("No response from journal"))??;
    /* send all descriptors */
    let builderbot_descriptors = builderbots
        .keys()
        .map(|desc| builderbot::Descriptor::clone(desc))
        .collect::<Vec<_>>();
    let drone_descriptors = drones
        .keys()
        .map(|desc| drone::Descriptor::clone(desc))
        .collect::<Vec<_>>();
    let pipuck_descriptors = pipucks
        .keys()
        .map(|desc| pipuck::Descriptor::clone(desc))
        .collect::<Vec<_>>();
    let descriptor_event = journal::Event::Descriptors(builderbot_descriptors, drone_descriptors, pipuck_descriptors);
    journal_requests_tx.send(journal::Action::Record(descriptor_event)).await
        .map_err(|_| anyhow::anyhow!("Could not send robot descriptors to journal"))?;
    /* set up the experiment on the builderbots */
    builderbots.iter()
        .map(|(desc, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = builderbot::Action::SetupExperiment(
                callback_tx, 
                desc.id.clone(),
                builderbot_software.clone(),
                journal_requests_tx.clone()
            );
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to BuilderBot"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from BuilderBot"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* set up the experiment on the pi-pucks */
    pipucks.iter()
        .map(|(desc, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = pipuck::Action::SetupExperiment(
                callback_tx,
                desc.id.clone(),
                pipuck_software.clone(),
                journal_requests_tx.clone()
            );
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to Pi-Puck"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from Pi-Puck"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
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
    pipucks.iter()
        .map(|(_, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = pipuck::Action::StartExperiment(callback_tx);
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to Pi-Puck"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from Pi-Puck"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
    /* start the builderbots */
    builderbots.iter()
        .map(|(_, instance)| {
            let (callback_tx, callback_rx) = oneshot::channel();
            let action = builderbot::Action::StartExperiment(callback_tx);
            async move {
                instance.action_tx.send(action).await
                    .map_err(|_| anyhow::anyhow!("Could not send action to BuilderBot"))?;
                callback_rx.await
                    .map_err(|_| anyhow::anyhow!("No response from BuilderBot"))?
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>().await?;
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
