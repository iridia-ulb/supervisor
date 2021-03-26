use futures::{Future, FutureExt, TryFutureExt, TryStreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::Ipv4Addr, path::PathBuf, time::Duration};
use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
use tokio_stream::StreamExt;
use crate::network::fernbedienung;
use crate::journal;
use crate::software;

#[derive(Debug)]
pub struct State {
    pub rpi: (Ipv4Addr, i32),
    pub actions: Vec<Action>,
}

pub enum Request {
    State(oneshot::Sender<State>),
    Execute(Action),
    ExperimentStart {
        software: software::Software,
        journal: mpsc::UnboundedSender<journal::Request>,
        callback: oneshot::Sender<Result<()>>
    },
    ExperimentStop,
}

pub type Sender = mpsc::UnboundedSender<Request>;
pub type Receiver = mpsc::UnboundedReceiver<Request>;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Halt Raspberry Pi")]
    RpiHalt,
    #[serde(rename = "Reboot Raspberry Pi")]
    RpiReboot,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Operation timed out")]
    Timeout,
    #[error("Could not send request")]
    RequestError,
    #[error("Did not receive response")]
    ResponseError,

    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn poll_rpi_link_strength(device: &fernbedienung::Device) -> Result<i32> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::time::timeout(Duration::from_secs(1), device.link_strength()).await
        .map_err(|_| Error::Timeout)
        .and_then(|inner| inner.map_err(|error| Error::FernbedienungError(error)))
}

pub async fn new(uuid: Uuid, mut arena_rx: Receiver, device: fernbedienung::Device) -> Uuid {
    let mut terminate: Option<oneshot::Sender<()>> = Default::default();
    let argos_task = futures::future::pending().left_future();
    tokio::pin!(argos_task);

    let poll_rpi_link_strength_task = poll_rpi_link_strength(&device);
    tokio::pin!(poll_rpi_link_strength_task);
    let mut rpi_link_strength = -100;

    loop {
        tokio::select! {
            /* poll the fernbedienung, exiting the main loop if we don't get a response */
            result = &mut poll_rpi_link_strength_task => match result {
                Ok(link_strength) => {
                    rpi_link_strength = link_strength;
                    poll_rpi_link_strength_task.set(poll_rpi_link_strength(&device));
                }
                Err(error) => {
                    log::warn!("Raspberry Pi on Pi-Puck {}: {}", uuid, error);
                    break;
                }
            },
            /* if ARGoS is running, keep forwarding stdout/stderr  */
            argos_result = &mut argos_task => {
                terminate = None;
                argos_task.set(futures::future::pending().left_future());
                log::info!("ARGoS terminated with {:?}", argos_result);
            },
            /* handle incoming requests */
            recv_request = arena_rx.recv() => match recv_request {
                None => break, /* tx handle dropped, exit the loop */
                Some(request) => match request {
                    Request::State(callback) => {
                        let state = State {
                            rpi: (device.addr, rpi_link_strength),
                            actions: vec![Action::RpiHalt, Action::RpiReboot]
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
                        Action::RpiHalt => {
                            if let Err(error) = device.halt().await {
                                log::error!("Halt failed: {}", error);
                            }
                            break;
                        }
                    },
                    Request::ExperimentStart{software, journal, callback} => {
                        match handle_experiment_start(uuid, &device, software, journal).await {
                            Ok((argos, terminate_tx)) => {
                                argos_task.set(argos.right_future());
                                terminate = Some(terminate_tx);
                                let _ = callback.send(Ok(()));
                            },
                            Err(error) => {
                                let _ = callback.send(Err(error));
                            }
                        }
                    },
                    Request::ExperimentStop => {
                        if let Some(terminate) = terminate.take() {
                            let _ = terminate.send(());
                        }
                        /* poll argos to completion */
                        let result = (&mut argos_task).await;
                        log::info!("ARGoS terminated with {:?}", result);
                        argos_task.set(futures::future::pending().left_future());
                    }
                }
            }
        }
    }
    /* return the uuid so that arena knows which robot terminated */
    uuid
}

async fn handle_experiment_start<'d>(uuid: Uuid,
                                     device: &'d fernbedienung::Device,
                                     software: software::Software,
                                     journal: mpsc::UnboundedSender<journal::Request>) 
    -> Result<(impl Future<Output = fernbedienung::Result<bool>> + 'd, oneshot::Sender<()>)> {
    /* extract the name of the config file */
    let (argos_config, _) = software.argos_config()?;
    let argos_config = argos_config.to_owned();
    /* get the relevant ip address of this machine */
    let message_router_addr = async {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect((device.addr, 80)).await?;
        socket.local_addr().map(|mut socket| {
            socket.set_port(4950);
            socket
        })
    }.await?;

    /* upload the control software */
    let software_upload_path = device.create_temp_dir()
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
            .map_ok(|_| path)
        ).await?;

    /* create a remote instance of ARGoS3 */
    let task = fernbedienung::Run {
        target: "argos3".into(),
        working_dir: software_upload_path.into(),
        args: vec![
            "--config".to_owned(), argos_config.to_owned(),
            "--router".to_owned(), message_router_addr.to_string(),
            "--id".to_owned(), uuid.to_string(),
        ],
    };

    /* channel for terminating ARGoS */
    let (terminate_tx, terminate_rx) = oneshot::channel();

    /* create future for running ARGoS */
    let argos_task_future = async move {
        /* channels for routing stdout and stderr to the journal */
        let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel();
        let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel();
        /* run argos remotely */
        let argos = device.run(task, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
        tokio::pin!(argos);
        loop {
            tokio::select! {
                Some(data) = stdout_rx.recv() => {
                    let message = journal::Robot::StandardOutput(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request) {
                        log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                    }
                },
                Some(data) = stderr_rx.recv() => {
                    let message = journal::Robot::StandardError(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request) {
                        log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
                    }
                },
                exit_status = &mut argos => break exit_status,
            }
        }
    };
    Ok((argos_task_future, terminate_tx)) 
}