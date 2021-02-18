use futures::{Future, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::Ipv4Addr, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use crate::network::fernbedienung;
use crate::journal;

#[derive(Debug)]
pub struct State {
    pub linux: Ipv4Addr, // rename to addr
    pub actions: Vec<Action>,
}

pub enum Request {
    State,
    Execute(Action),
    Upload(crate::software::Software),
    ExperimentStart(mpsc::UnboundedSender<journal::Request>),
    ExperimentStop,
}

/*
 note: this has similar semantics to a result
 although the implications of the monomorphization could make the code
  complicated or even incorrect
*/
#[derive(Debug)]
pub enum Response {
    State(State),
    Ok,
    Error(Error),
}

pub type Sender = mpsc::UnboundedSender<(Request, oneshot::Sender<Response>)>;
pub type Receiver = mpsc::UnboundedReceiver<(Request, oneshot::Sender<Response>)>;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Shutdown RPi")]
    RpiShutdown,
    #[serde(rename = "Reboot RPi")]
    RpiReboot,
    /*
    #[serde(rename = "Identify")]
    Identify,
    */
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),

    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),

    #[error("Could not send request")]
    RequestError,

    #[error("Did not receive response")]
    ResponseError,

    #[error("Invalid control software")]
    InvalidControlSoftware,
}

pub async fn new(uuid: Uuid, rx: Receiver, device: fernbedienung::Device) -> (Uuid, Ipv4Addr) {
    let mut argos_path: Option<String> = Default::default();
    let mut argos_config: Option<String> = Default::default();

    let (device_task, device_interface, device_addr) = device.split();
    let request_task = async move {
        let mut requests = UnboundedReceiverStream::new(rx);
        
        let mut processes : FuturesUnordered<_> = Default::default();
        let mut forward : FuturesUnordered<_> = Default::default();
        
        loop {
            tokio::select! {
                Some((request, callback)) = requests.next() => {
                    let response = match request {
                        Request::State => {
                            let state = State {
                                linux: device_addr,
                                actions: vec![Action::RpiShutdown, Action::RpiReboot]
                            };
                            Response::State(state)
                        }
                        Request::Execute(action) => match action {
                            Action::RpiReboot => {
                                if let Err(error) = device_interface.clone().reboot().await {
                                    log::error!("Reboot failed: {}", error);
                                }
                                Response::Ok
                                //break;
                            },
                            Action::RpiShutdown => {
                                if let Err(error) = device_interface.clone().shutdown().await {
                                    log::error!("Shut down failed: {}", error);
                                }
                                Response::Ok
                                //break;
                            }
                        },
                        Request::Upload(software) => {
                            argos_config = software.argos_config()
                                .map(|(argos_config, _)| argos_config.to_owned())
                                .ok();
                            match device_interface.clone().create_temp_dir().await {
                                Ok(path) => {
                                    let upload_result = software.0.into_iter()
                                        .map(|(filename, contents)| {
                                            device_interface.clone().upload(&path, filename, contents)
                                        })
                                        .collect::<FuturesUnordered<_>>()
                                        .collect::<Result<Vec<_>, _>>().await;
                                    match upload_result {
                                        Ok(_) => {
                                            argos_path.get_or_insert(path);
                                            Response::Ok
                                        },
                                        Err(error) => Response::Error(Error::FernbedienungError(error))
                                    }
                                }
                                Err(error) => Response::Error(Error::FernbedienungError(error))
                            }
                        },
                        // step 1, connect this method
                        // step 2, modify argos to take a controller and ip address to the router
                        // step 3, set .argos file so that control terminates after X ticks
                        // step 4, add channel for issuing signal when experiment stop is requested
                        // step 5, forward standard input and standard output to journal
                        Request::ExperimentStart(journal_requests_tx) => {
                            match argos_config.take() {
                                Some(argos_config) => match argos_path.take() {
                                    Some(argos_path) => {
                                        let task = fernbedienung::process::Run {
                                            target: "argos3".into(),
                                            working_dir: argos_path.into(),
                                            args: vec!["-c".into(), argos_config],
                                        };
                                        let (stdout_tx, stdout_rx) = mpsc::unbounded_channel();
                                        let (stderr_tx, stderr_rx) = mpsc::unbounded_channel();

                                        let argos = device_interface
                                            .clone()
                                            .run(task, None, None, Some(stdout_tx), Some(stderr_tx));
                                        processes.push(argos);                                            
                                        forward.push(async move {
                                            let mut stdout_stream = UnboundedReceiverStream::new(stdout_rx);
                                            let mut stderr_stream = UnboundedReceiverStream::new(stderr_rx);
                                            loop {
                                                tokio::select! {
                                                    Some(data) = stdout_stream.next() => {
                                                        let message = journal::Robot::StandardOutput(data);
                                                        let event = journal::Event::Robot(uuid, message);
                                                        let request = journal::Request::Event(event);
                                                        if let Err(error) = journal_requests_tx.send(request) {
                                                            log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                                                        }
                                                    },
                                                    Some(data) = stderr_stream.next() => {
                                                        let message = journal::Robot::StandardError(data);
                                                        let event = journal::Event::Robot(uuid, message);
                                                        let request = journal::Request::Event(event);
                                                        if let Err(error) = journal_requests_tx.send(request) {
                                                            log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
                                                        }
                                                    },
                                                    else => break,
                                                }
                                            }
                                        });
                                        Response::Ok
                                    }
                                    None => Response::Error(Error::InvalidControlSoftware),
                                },
                                None => Response::Error(Error::InvalidControlSoftware),
                            }
                        },
                        Request::ExperimentStop => {
                            Response::Ok
                        }
                    };
                    if let Err(response) = callback.send(response) {
                        log::error!("Could not respond with {:?}", response);
                    }
                },
                Some(exit_success) = processes.next() => {
                    // remember to revert sysfstrig changes when done
                    log::info!("ARGoS terminated with {:?}", exit_success);
                },
                Some(_) = forward.next() => {
                    log::info!("Forwarding of standard output/error completed");
                }
            }
        }
    };
    /* select here instead of join since if one future completes the
       other should be dropped */
    tokio::select! {
        _ = device_task => {},
        _ = request_task => {},
    }
    (uuid, device_addr)
}
