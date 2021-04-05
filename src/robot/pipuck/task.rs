use bytes::Bytes;
use futures::{Future, FutureExt, StreamExt, TryFutureExt, TryStreamExt, future::Either, stream::{FuturesOrdered, FuturesUnordered}};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use std::{net::{Ipv4Addr, SocketAddr}, path::PathBuf, time::Duration};
use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
use crate::network::fernbedienung;
use crate::journal;
use crate::software;

// Info about reading the Pi-Puck battery level here:
// https://github.com/yorkrobotlab/pi-puck-packages/blob/master/pi-puck-utils/pi-puck-battery
/*
root@raspberrypi0-wifi:/sys/devices/platform/soc/20804000.i2c/i2c-1/i2c-11/11-0048/iio:device1# cat name 
ads1015
root@raspberrypi0-wifi:/sys/devices/platform/soc/20804000.i2c/i2c-1/i2c-11/11-0048/iio:device1# cat in_voltage0_raw
1042
*/

#[derive(Debug)]
pub struct State {
    pub rpi: (Ipv4Addr, i32),
    pub camera: Vec<Bytes>,
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
    #[serde(rename = "Start Camera")]
    StartCamera,
    #[serde(rename = "Stop Camera")]
    StopCamera,
}

//fswebcam -d /dev/video0 --input 0 --palette YUYV --resolution 640x480 --no-banner --jpeg -1 --save -


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
    FetchError(#[from] reqwest::Error),
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn poll_rpi_link_strength(device: &fernbedienung::Device) -> Result<i32> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::time::timeout(Duration::from_secs(2), device.link_strength()).await
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

    let mut rpi_camera_stream = futures::stream::pending().left_stream();
    let rpi_camera_task = futures::future::pending().left_future();
    tokio::pin!(rpi_camera_task);
    let mut rpi_camera_frames = Vec::new();

    loop {
        tokio::select! {
            Some(frames) = rpi_camera_stream.next() => {
                rpi_camera_frames = frames;
            }
            /* poll the fernbedienung, exiting the main loop if we don't get a response */
            result = &mut poll_rpi_link_strength_task => match result {
                Ok(link_strength) => {
                    rpi_link_strength = link_strength;
                    poll_rpi_link_strength_task.set(poll_rpi_link_strength(&device));
                }
                Err(error) => {
                    log::warn!("Polling link strength failed for Pi-Puck {}: {}", uuid, error);
                    break;
                }
            },
            /* if ARGoS is running, keep forwarding stdout/stderr  */
            argos_result = &mut argos_task => {
                terminate = None;
                argos_task.set(futures::future::pending().left_future());
                log::info!("ARGoS terminated with {:?}", argos_result);
            },
            /* if the streaming process terminates, disable streaming */
            rpi_camera_result = &mut rpi_camera_task => {
                rpi_camera_task.set(futures::future::pending().left_future());
                rpi_camera_frames.clear();
                match rpi_camera_result {
                    Ok(_) => log::info!("Camera stream stopped"),
                    Err(error) => log::warn!("Camera stream stopped: {}", error),
                }
            },
            /* handle incoming requests */
            recv_request = arena_rx.recv() => match recv_request {
                None => break, /* tx handle dropped, exit the loop */
                Some(request) => match request {
                    Request::State(callback) => {
                        let state = State {
                            rpi: (device.addr, rpi_link_strength),
                            actions: vec![Action::RpiHalt, Action::RpiReboot, match *rpi_camera_task {
                                Either::Left(_) => Action::StartCamera,
                                Either::Right(_) => Action::StopCamera
                            }],
                            camera: rpi_camera_frames.clone(),
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
                        Action::StartCamera => {
                            if let Either::Left(_) = *rpi_camera_task {
                                let (task, stop_tx, stream_rx) = handle_stream_start(&device);
                                rpi_camera_stream = ReceiverStream::new(stream_rx).right_stream();
                                rpi_camera_task.set(task.right_future());
                            }
                        },
                        Action::StopCamera => {
                            if let Either::Right(_) = *rpi_camera_task {
                                rpi_camera_task.set(futures::future::pending().left_future());
                                rpi_camera_frames.clear();
                            }
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

fn handle_stream_start<'d>(device: &'d fernbedienung::Device)
    -> (impl Future<Output = Result<()>> + 'd, oneshot::Sender<()>, mpsc::Receiver<Vec<Bytes>>) {

    let cameras = [("/dev/video0", 1280u16, 720u16)];
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let (stream_tx, stream_rx) = mpsc::channel::<Vec<Bytes>>(2);

    // pass in port/stop_tx somehow?
    // make port part of camera?

    let mjpeg_processes = cameras.iter()
        .enumerate()
        .map(|(index, &(camera, width, height))| {
            let port = 8000 + index as u16;
            let process = fernbedienung::Process {
                target: "/home/mallwright/Workspace/mjpg-streamer/build/mjpg_streamer".into(),
                working_dir: "/tmp".into(),
                args: vec![
                    "-i".to_owned(),
                    format!("plugins/input_uvc/input_uvc.so -d {} -r {}x{} -n", camera, width, height),
                    "-o".to_owned(),
                    format!("plugins/output_http/output_http.so -p {} -l {}", port, device.addr)
                ],
            };
            device.run(process, None, None, None, None)
                .map_err(|e| Error::FernbedienungError(e))
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>();
    
    // mjpeg_processes future<output = pipuck::Result<Vec<bool>>>

    let task = futures::future::try_join(mjpeg_processes, async move {
        /* sleep a bit while mjpeg_stream starts */
        tokio::time::sleep(Duration::from_millis(500)).await;
        let sockets = [SocketAddr::from(([127,0,0,1], 8000))];       
        loop {
            let reqwest_frames = sockets.iter()
                .map(|socket| {
                    reqwest::get(format!("http://{}/?action=snapshot", socket))
                        .and_then(|response| response.bytes())
                })
                .collect::<FuturesOrdered<_>>()
                .try_collect::<Vec<_>>();
            tokio::select! {
                _ = &mut stop_rx => break Ok(()),
                reqwest_result = reqwest_frames => match reqwest_result {
                    Ok(frames) => {
                        if let Err(_) = stream_tx.send(frames).await {
                            break Ok(());
                        }
                    },
                    Err(error) => break Err(Error::FetchError(error)),
                },
            }
        }
    }).map_ok(|_| ());
    (task, stop_tx, stream_rx)
}

async fn handle_experiment_start<'d>(uuid: Uuid,
                                     device: &'d fernbedienung::Device,
                                     software: software::Software,
                                     journal: mpsc::UnboundedSender<journal::Request>) 
    -> Result<(impl Future<Output = fernbedienung::Result<()>> + 'd, oneshot::Sender<()>)> {
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
            .try_collect::<Vec<_>>()
            .map_ok(|_| path)
        ).await?;

    /* create a remote instance of ARGoS3 */
    let process = fernbedienung::Process {
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
        let argos = device.run(process, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
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