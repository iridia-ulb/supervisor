use futures::{Future, TryFutureExt, TryStreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::{Ipv4Addr, SocketAddr}, path::PathBuf, sync::Arc};
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
    State(oneshot::Sender<State>),
    Execute(Action),
    Upload(crate::software::Software, oneshot::Sender<Result<()>>),
    ExperimentStart(SocketAddr, mpsc::UnboundedSender<journal::Request>, oneshot::Sender<Result<()>>),
    ExperimentStop,
}

pub type Sender = mpsc::UnboundedSender<Request>;
pub type Receiver = mpsc::UnboundedReceiver<Request>;

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

pub type Result<T> = std::result::Result<T, Error>;

pub async fn new(uuid: Uuid, mut arena_rx: Receiver, device: fernbedienung::Device) -> (Uuid, Ipv4Addr) {
    let mut argos_path: Option<String> = Default::default();
    let mut argos_config: Option<String> = Default::default();
    let mut processes : FuturesUnordered<_> = Default::default();
    let mut forward : FuturesUnordered<_> = Default::default();
    loop {
        tokio::select! {
            Some(request) = arena_rx.recv() => {
                match request {
                    Request::State(callback) => {
                        let state = State {
                            linux: device.addr,
                            actions: vec![Action::RpiShutdown, Action::RpiReboot]
                        };
                        let _ = callback.send(state);
                    }
                    Request::Execute(action) => match action {
                        Action::RpiReboot => {
                            if let Err(error) = device.reboot().await {
                                log::error!("Reboot failed: {}", error);
                            }
                            break;
                        },
                        Action::RpiShutdown => {
                            if let Err(error) = device.shutdown().await {
                                log::error!("Shutdown failed: {}", error);
                            }
                            break;
                        }
                    },
                    Request::Upload(software, callback) => {
                        argos_config = software.argos_config()
                            .map(|(argos_config, _)| argos_config.to_owned())
                            .ok();
                        let upload_result = device.create_temp_dir()
                            .map_err(|error| Error::FernbedienungError(error))
                            .and_then(|path: String| software.0.into_iter()
                                .map(|(filename, contents)| {
                                    let path = PathBuf::from(&path);
                                    let filename = PathBuf::from(&filename);
                                    device.upload(path, filename, contents)
                                })
                                .collect::<FuturesUnordered<_>>()
                                .map_err(|error| Error::FernbedienungError(error))
                                .collect::<Result<Vec<_>>>()
                                .map_ok(|vec| path)
                            ).await;
                        let _ = callback.send(upload_result.map(|path| {
                            argos_path.get_or_insert(path);
                        }));
                    },
                    Request::ExperimentStart(message_router_addr, journal_requests_tx, callback) => {
                        let response = match argos_config.take() {
                            None => Err(Error::InvalidControlSoftware),
                            Some(argos_config) => match argos_path.take() {
                                None => Err(Error::InvalidControlSoftware),
                                Some(argos_path) => {
                                    let task = fernbedienung::Run {
                                        target: "argos3".into(),
                                        working_dir: argos_path.into(),
                                        args: vec![
                                            "--config".to_owned(), argos_config,
                                            "--router".to_owned(), message_router_addr.to_string(),
                                            "--id".to_owned(), uuid.to_string(),
                                        ],
                                    };
                                    let (stdout_tx, stdout_rx) = mpsc::unbounded_channel();
                                    let (stderr_tx, stderr_rx) = mpsc::unbounded_channel();

                                    let argos = device.run(task, None, None, Some(stdout_tx), Some(stderr_tx));
                                    processes.push(argos);
                                    // the following could be refactored under the device interface
                                    forward.push(async move {
                                        let mut stdout_stream = UnboundedReceiverStream::new(stdout_rx);
                                        let mut stderr_stream = UnboundedReceiverStream::new(stderr_rx);
                                        loop {
                                            tokio::select! {
                                                Some(data) = stdout_stream.next() => {
                                                    let message = journal::Robot::StandardOutput(data);
                                                    let event = journal::Event::Robot(uuid, message);
                                                    let request = journal::Request::Record(event);
                                                    if let Err(error) = journal_requests_tx.send(request) {
                                                        log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                                                    }
                                                },
                                                Some(data) = stderr_stream.next() => {
                                                    let message = journal::Robot::StandardError(data);
                                                    let event = journal::Event::Robot(uuid, message);
                                                    let request = journal::Request::Record(event);
                                                    if let Err(error) = journal_requests_tx.send(request) {
                                                        log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
                                                    }
                                                },
                                                else => break,
                                            }
                                        }
                                    });
                                    Ok(())
                                }
                            },
                        };
                        let _ = callback.send(response);
                    },
                    Request::ExperimentStop => {}
                }
            },
            Some(exit_success) = processes.next() => {
                // remember to revert sysfstrig changes when done
                /* TODO, send exit_success to arena so that experiment is automatically ended
                    when ARGoS terminates? */
                log::info!("ARGoS terminated with {:?}", exit_success);
            },
            Some(_) = forward.next() => {
                log::info!("Forwarding of standard output/error completed");
            },
            else => break,
        }
    }
    (uuid, device.addr)
}
