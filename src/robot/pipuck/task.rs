use futures::{Future, TryFutureExt, TryStreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::{Ipv4Addr, SocketAddr}, path::PathBuf, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
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
    ExperimentStart(mpsc::UnboundedSender<journal::Request>, oneshot::Sender<Result<()>>),
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

    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn poll_fernbedienung(device: &fernbedienung::Device) {
    while let Ok(Ok(true)) = tokio::time::timeout(Duration::from_millis(500),  device.ping()).await {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn new(uuid: Uuid, mut arena_rx: Receiver, device: fernbedienung::Device) -> Uuid {
    let mut argos_path: Option<String> = Default::default();
    let mut argos_config: Option<String> = Default::default();
    let mut processes : FuturesUnordered<_> = Default::default();
    let mut forward : FuturesUnordered<_> = Default::default();
    let mut terminate: Option<oneshot::Sender<()>> = Default::default();

    let poll_fernbedienung_task = poll_fernbedienung(&device);
    tokio::pin!(poll_fernbedienung_task);

    loop {
        tokio::select! {
            /* poll the fernbedienung, breaking the loop if we don't get a response */
            _ = &mut poll_fernbedienung_task => break,
            /* handle requests */
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
                    Request::ExperimentStart(journal_requests_tx, callback) => {
                        let message_router_addr = async {
                            /* hack for getting the local ip address */
                            let socket = UdpSocket::bind("0.0.0.0:0").await?;
                            socket.connect((device.addr, 80)).await?;
                            socket.local_addr().map(|mut socket| {
                                socket.set_port(4950);
                                socket
                            })
                        }.await;
                        let response = match message_router_addr {
                            Err(e) => Err(Error::IoError(e)),
                            Ok(message_router_addr) => match argos_config.take() {
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
                                        let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel();
                                        let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel();
                                        let (terminate_tx, terminate_rx) = oneshot::channel();
                                        terminate = Some(terminate_tx);
    
                                        let argos = device.run(task, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
                                        processes.push(argos);
                                        // the following could be refactored under the device interface
                                        forward.push(async move {
                                            loop {
                                                tokio::select! {
                                                    Some(data) = stdout_rx.recv() => {
                                                        let message = journal::Robot::StandardOutput(data);
                                                        let event = journal::Event::Robot(uuid, message);
                                                        let request = journal::Request::Record(event);
                                                        if let Err(error) = journal_requests_tx.send(request) {
                                                            log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                                                        }
                                                    },
                                                    Some(data) = stderr_rx.recv() => {
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
                            }
                        };
                        let _ = callback.send(response);
                    },
                    Request::ExperimentStop => {
                        if let Some(terminate) = terminate.take() {
                            let _ = terminate.send(());
                        }
                    }
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
    uuid
}
