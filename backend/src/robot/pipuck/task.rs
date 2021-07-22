use bytes::Bytes;
use futures::{Future, FutureExt, StreamExt, TryFutureExt, TryStreamExt, future::Either, stream::{FuturesOrdered, FuturesUnordered}};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use std::{net::Ipv4Addr, path::PathBuf, time::Duration};
use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
use crate::network::fernbedienung;
use crate::journal;
use crate::software;

//const PIPUCK_BATT_FULL_MV: f32 = 4050.0;
//const PIPUCK_BATT_EMPTY_MV: f32 = 3500.0;
const PIPUCK_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[];

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
    pub cameras: Vec<Bytes>,
    pub actions: Vec<Action>,
    pub kernel_messages: Option<String>,
}

pub enum Request {
    AssociateFernbedienung(fernbedienung::Device),

    // when this message is recv, all updates are sent and then
    // updates are sent only on changes
    SetUpdateEndpoint(mpsc::Sender<Update>),



    ExperimentStart {
        software: software::Software,
        journal: mpsc::Sender<journal::Request>,
        callback: oneshot::Sender<Result<()>>
    },
    ExperimentStop,
}

pub type Sender = mpsc::Sender<Request>;
pub type Receiver = mpsc::Receiver<Request>;

//
#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Halt Raspberry Pi")]
    RpiHalt,
    #[serde(rename = "Reboot Raspberry Pi")]
    RpiReboot,
    #[serde(rename = "Start camera stream")]
    StartCameraStream,
    #[serde(rename = "Stop camera stream")]
    StopCameraStream,
    #[serde(rename = "Get kernel messages")]
    GetKernelMessages,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Operation timed out")]
    Timeout,
    #[error("Could not send request")]
    RequestError,
    #[error("Did not receive response")]
    ResponseError,

    // drone will also have a 
    #[error("A Fernbedienung instance has not been associated")]
    FernbedienungNotAssociated,

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

// theory of operation
// how are changes detected? in part by polling -- these changes need to be sent to the
// webui -- when a change in state occurs this should be pushed via a channel to the webui

// perhaps make two sub-actors -- one for handling xbee requests, when it has been added,
// the other for handling ferbedienung requests

// let mut fernbedienung: Option<Arc<fernbedienung::Device>> = None;
// let poll_upcore_link_strength_task = future::pending().left_future();


mod argos {
    enum Request {
        Start,
        Stop
    }
}

enum FernbedienungRequest {
    StreamCameras(mpsc::Sender<Vec<Bytes>>),
    StartArgos,
    StopArgos,

}

async fn fernbedienung(
    device: fernbedienung::Device,
    rx: mpsc::Receiver<FernbedienungRequest>
) -> anyhow::Result<()> {

    Ok(())
}

// arena sends drone a request StreamUpdates(mpsc::Sender<Updates>)
// sender can be cloned and passed into fernbedienung/xbee tasks
// the webui is the only recieving end

// TODO: I think actions and requests can be merged

pub async fn new(mut request_rx: Receiver) {
    let fernbedienung_task = futures::future::pending().left_future();
    let mut fernbedienung_tx = Option::default();
    tokio::pin!(fernbedienung_task);

    loop {
        tokio::select! {
            Some(request) = request_rx.recv() => match request {
                Request::AssociateFernbedienung(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    let state_change = StateChange::FernbedienungConnection(Some(device.addr));
                    updates_tx.send(state_change).await;
                    fernbedienung_tx = Some(tx);
                    fernbedienung_task.set(fernbedienung(device, rx).right_future());

                },
                Request::RefreshState => {
                    let updates = vec![
                        Update::FernbedienungConnection(None),
                        Update::FernbedienungSignal(0),
                        Update::Cameras(vec![]),
                        Update::Actions(vec![])
                    ];
                    let result = updates
                        .into_iter()
                        .map(|update| updates_tx.send(update))
                        .collect::<FuturesUnordered<_>>()
                        .try_collect::<Vec<_>>().await;
                    if let Err(error) = result {
                        log::error!("Could not send state update")
                    }
                },
                Request::Execute(action) => todo!(),
                Request::ExperimentStart { software, journal, callback } => todo!(),
                Request::ExperimentStop => todo!(),
            },
            result = &mut fernbedienung_task => {
                fernbedienung_tx = None;
                fernbedienung_task.set(futures::future::pending().left_future());
                if let Err(error) = result {
                    log::info!("Fernbedienung task terminated: {}", error);
                }
            }
        }
    }
}


pub async fn old_code_new(mut arena_rx: Receiver) {
    let mut argos_stop_tx = None;
    let argos_task = futures::future::pending().left_future();
    tokio::pin!(argos_task);

    let poll_rpi_link_strength_task = poll_rpi_link_strength(&device);
    tokio::pin!(poll_rpi_link_strength_task);
    let mut rpi_link_strength = -100;

    let mut kernel_messages = None;

    let mut rpi_camera_stream = futures::stream::pending().left_stream();
    let mut rpi_camera_stream_stop_tx = None;
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
                argos_stop_tx = None;
                argos_task.set(futures::future::pending().left_future());
                log::info!("ARGoS terminated with {:?}", argos_result);
            },
            /* clean up for when the streaming process terminates */
            rpi_camera_result = &mut rpi_camera_task => {
                rpi_camera_task.set(futures::future::pending().left_future());
                rpi_camera_frames.clear();
                match rpi_camera_result {
                    /* since we use the terminate signal to shutdown mjpg_streamer, report
                       AbnormalTerminationError as not an error */
                    Ok(_) | Err(Error::FernbedienungError(fernbedienung::Error::AbnormalTerminationError)) => 
                        log::info!("Camera stream stopped"),
                    Err(error) =>
                        log::warn!("Camera stream stopped: {}", error),
                }
            },
            /* handle incoming requests */
            recv_request = arena_rx.recv() => match recv_request {
                None => break, /* tx handle dropped, exit the loop */
                Some(request) => match request {
                    Request::State(callback) => {
                        let state = State {
                            rpi: (device.addr, rpi_link_strength),
                            actions: vec![
                                Action::RpiHalt, Action::RpiReboot, Action::GetKernelMessages, 
                                match *rpi_camera_task {
                                    Either::Left(_) => Action::StartCameraStream,
                                    Either::Right(_) => Action::StopCameraStream
                                }
                            ],
                            cameras: rpi_camera_frames.clone(),
                            kernel_messages: kernel_messages.take(),
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
                        Action::GetKernelMessages => {
                            match device.kernel_messages().await {
                                Ok(messages) => kernel_messages = Some(messages),
                                Err(error) => log::error!("Could not get kernel messages: {}", error),
                            };
                        }
                        Action::StartCameraStream => {
                            if let Either::Left(_) = *rpi_camera_task {
                                let (task, stop_tx, stream_rx) = 
                                    handle_stream_start(&device, PIPUCK_CAMERAS_CONFIG);
                                rpi_camera_stream_stop_tx = Some(stop_tx);
                                rpi_camera_stream = ReceiverStream::new(stream_rx).right_stream();
                                rpi_camera_task.set(task.right_future());
                                log::info!("Camera stream started");
                            }
                            else {
                                log::warn!("Camera stream already started");
                            }
                        },
                        Action::StopCameraStream => {
                            if let Either::Right(_) = *rpi_camera_task {
                                if let Some(stop_tx) = rpi_camera_stream_stop_tx.take() {
                                    let _ = stop_tx.send(());
                                }
                            }
                            else {
                                log::warn!("Camera stream has not been started");
                            }
                        }
                    },
                    // modify experiment start to use a mpsc channel to send ARGoS started/stopped
                    // events back to the arena. The stop event should be sent when ARGoS terminates
                    Request::ExperimentStart{software, journal, callback} => {
                        match handle_experiment_start(uuid, &device, software, journal).await {
                            Ok((argos, stop_tx)) => {
                                argos_task.set(argos.right_future());
                                argos_stop_tx = Some(stop_tx);
                                let _ = callback.send(Ok(()));
                            },
                            Err(error) => {
                                let _ = callback.send(Err(error));
                            }
                        }
                    },
                    Request::ExperimentStop => {
                        if let Some(stop_tx) = argos_stop_tx.take() {
                            let _ = stop_tx.send(());
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

fn handle_stream_start<'d>(device: &'d fernbedienung::Device, configs: &'static [(&str, u16, u16, u16)])
    -> (impl Future<Output = Result<()>> + 'd, oneshot::Sender<()>, mpsc::Receiver<Vec<Bytes>>) {
    let (processes, stop_txs) : (FuturesUnordered<_>, Vec<_>)  = configs.iter()
        .map(|(camera, width, height, port)| {
            let process_request = fernbedienung::Process {
                target: "mjpg_streamer".into(),
                working_dir: None,
                args: vec![
                    "-i".to_owned(),
                    format!("input_uvc.so -d {} -r {}x{} -n", camera, width, height),
                    "-o".to_owned(),
                    format!("output_http.so -p {} -l {}", port, device.addr)
                ],
            };
            let (stop_tx, stop_rx) = oneshot::channel::<()>();
            let process = device.run(process_request, Some(stop_rx), None, None, None)
                .map_err(|e| Error::FernbedienungError(e));
            (process, stop_tx)
        }).unzip();

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let (stream_tx, stream_rx) = mpsc::channel::<Vec<Bytes>>(2);
    let task = futures::future::try_join(processes.try_collect::<Vec<_>>(), async move {
        /* sleep a bit while mjpeg_stream starts */
        tokio::time::sleep(Duration::from_millis(500)).await;
        /* poll for frames */
        loop {
            let reqwest_frames = configs.iter()
                .map(|&(_, _, _, port)| {
                    reqwest::get(format!("http://{}:{}/?action=snapshot", device.addr, port))
                        .and_then(|response| response.bytes())
                })
                .collect::<FuturesOrdered<_>>()
                .try_collect::<Vec<_>>();
            tokio::select! {
                _ = &mut stop_rx => {
                    break stop_txs.into_iter()
                        .map(|stop_tx| stop_tx.send(()).map_err(|_| Error::RequestError))
                        .collect::<Result<Vec<_>>>()
                        .map(|_| ())
                },
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
                                     journal: mpsc::Sender<journal::Request>) 
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
        working_dir: Some(software_upload_path.into()),
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
        let (stdout_tx, mut stdout_rx) = mpsc::channel(8);
        let (stderr_tx, mut stderr_rx) = mpsc::channel(8);
        /* run argos remotely */
        let argos = device.run(process, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
        tokio::pin!(argos);
        loop {
            tokio::select! {
                Some(data) = stdout_rx.recv() => {
                    let message = journal::Robot::StandardOutput(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request).await {
                        log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                    }
                },
                Some(data) = stderr_rx.recv() => {
                    let message = journal::Robot::StandardError(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request).await {
                        log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
                    }
                },
                exit_status = &mut argos => break exit_status,
            }
        }
    };
    Ok((argos_task_future, terminate_tx)) 
}