use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{convert::TryInto, num::TryFromIntError, path::PathBuf, time::Duration};
use std::result::Result;

use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
use futures::{Future, FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt, stream::{FuturesUnordered}};
use tokio_stream;

use crate::network::fernbedienung;
use crate::network::fernbedienung_ext::MjpegStreamerStream;
use crate::journal;
use crate::software;

pub use shared::pipuck::Update;

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

pub enum Request {
    AssociateFernbedienung(fernbedienung::Device),

    // when this message is recv, all updates are sent and then
    // updates are sent only on changes
    SetUpdateChannel(mpsc::Sender<Update>),



    ExperimentStart {
        software: software::Software,
        journal: mpsc::Sender<journal::Request>,
        callback: oneshot::Sender<Result<(), Error>>
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

// theory of operation
// how are changes detected? in part by polling -- these changes need to be sent to the
// webui -- when a change in state occurs this should be pushed via a channel to the webui

// perhaps make two sub-actors -- one for handling xbee requests, when it has been added,
// the other for handling ferbedienung requests

// let mut fernbedienung: Option<Arc<fernbedienung::Device>> = None;
// let poll_upcore_link_strength_task = future::pending().left_future();

enum FernbedienungRequest {
    SetUpdateChannel(mpsc::Sender<Update>),
    StartCameraStream,
    StopCameraStream,
    //StartArgos,
    //StopArgos,

}

fn link_strength_stream<'dev>(
    device: &'dev fernbedienung::Device
) -> impl Stream<Item = Result<u64, String>> + 'dev {
    async_stream::stream! {
        loop {
            let strength: Result<u64, String> = device.link_strength().await
                .map_err(|error| error.to_string())
                .and_then(|value| value.try_into()
                    .map_err(|error: TryFromIntError| error.to_string()));
            yield strength;
        }
    }
}

async fn fernbedienung(
    device: fernbedienung::Device,
    mut rx: mpsc::Receiver<FernbedienungRequest>
) -> anyhow::Result<()> {
    /* link strength */
    let link_strength_stream = link_strength_stream(&device)
        .map(Update::FernbedienungSignal);
    let link_strength_stream_throttled =
        tokio_stream::StreamExt::throttle(link_strength_stream, Duration::from_millis(1000));
    tokio::pin!(link_strength_stream_throttled);
    /* handle for the camera stream */
    let mut cameras_stream: tokio_stream::StreamMap<String, _> =
        tokio_stream::StreamMap::new();
    let mut updates_tx: Option<mpsc::Sender<Update>>
        = Default::default();
    loop {
        tokio::select! {
            Some((camera, result)) = cameras_stream.next() => {
                let result: reqwest::Result<bytes::Bytes> = result;
                if let Some(ref updates_tx) = updates_tx {
                    let update = Update::Camera(camera, result.map_err(|e| e.to_string()));
                    let _ = updates_tx.send(update).await;
                }
            },
            Some(update) = link_strength_stream_throttled.next() => {
                if let Some(ref updates_tx) = updates_tx {
                    let _ = updates_tx.send(update).await;
                }
            },
            recv = rx.recv() => match recv {
                Some(request) => match request {
                    FernbedienungRequest::SetUpdateChannel(channel) => {
                        // TODO revisit the logic here, what updates need to be sent?
                        let updates = vec![];
                        let _ = updates
                            .into_iter()
                            .map(|update| channel.send(update))
                            .collect::<FuturesUnordered<_>>()
                            .try_collect::<Vec<_>>().await;
                        updates_tx.replace(channel);
                    },
                    FernbedienungRequest::StartCameraStream => {
                        cameras_stream.clear();
                        for &(camera, width, height, port) in PIPUCK_CAMERAS_CONFIG {
                            let stream = MjpegStreamerStream::new(&device, camera, width, height, port);
                            let stream = tokio_stream::StreamExt::throttle(stream, Duration::from_millis(200));
                            cameras_stream.insert(camera.to_owned(), Box::pin(stream));
                        }
                    },
                    FernbedienungRequest::StopCameraStream => {
                        cameras_stream.clear();
                    }
                },
                None => break,
            }
        }
    }
    Ok(())
}

// arena sends drone a request StreamUpdates(mpsc::Sender<Updates>)
// sender can be cloned and passed into fernbedienung/xbee tasks
// the webui is the only recieving end

// TODO: I think actions and requests can be merged

pub async fn new(mut request_rx: Receiver) {
    let fernbedienung_task = futures::future::pending().left_future();
    let mut fernbedienung_tx = Option::default();
    let mut updates_tx: Option<mpsc::Sender<Update>> = Default::default();
    tokio::pin!(fernbedienung_task);

    loop {
        tokio::select! {
            Some(request) = request_rx.recv() => match request {
                Request::AssociateFernbedienung(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    if let Some(ref updates_tx) = updates_tx {
                        tx.send(FernbedienungRequest::SetUpdateChannel(updates_tx.clone())).await;
                    }
                    fernbedienung_tx = Some(tx);
                    fernbedienung_task.set(fernbedienung(device, rx).right_future());
                },
                Request::SetUpdateChannel(tx) => {
                    /* send all update variants (not associated with fernbedienung) */
                    /* add the channel to the fernbedienung process if it is active */                    
                    if let Some(ref fernbedienung_tx) = fernbedienung_tx {
                        let request = FernbedienungRequest::SetUpdateChannel(tx.clone());
                        let _ = fernbedienung_tx.send(request).await;
                    }
                    updates_tx.replace(tx);
                },
                Request::ExperimentStart { software, journal, callback } => log::warn!("not implemented"),
                Request::ExperimentStop => log::warn!("not implemented"),
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


// async fn handle_experiment_start<'d>(uuid: Uuid,
//                                      device: &'d fernbedienung::Device,
//                                      software: software::Software,
//                                      journal: mpsc::Sender<journal::Request>) 
//     -> Result<(impl Future<Output = fernbedienung::Result<()>> + 'd, oneshot::Sender<()>), Error> {
//     /* extract the name of the config file */
//     let (argos_config, _) = software.argos_config()?;
//     let argos_config = argos_config.to_owned();
//     /* get the relevant ip address of this machine */
//     let message_router_addr = async {
//         let socket = UdpSocket::bind("0.0.0.0:0").await?;
//         socket.connect((device.addr, 80)).await?;
//         socket.local_addr().map(|mut socket| {
//             socket.set_port(4950);
//             socket
//         })
//     }.await?;

//     /* upload the control software */
//     let software_upload_path = device.create_temp_dir()
//         .map_err(|error| Error::FernbedienungError(error))
//         .and_then(|path: String| software.0.into_iter()
//             .map(|(filename, contents)| {
//                 let path = PathBuf::from(&path);
//                 let filename = PathBuf::from(&filename);
//                 device.upload(path, filename, contents)
//             })
//             .collect::<FuturesUnordered<_>>()
//             .map_err(|error| Error::FernbedienungError(error))
//             .try_collect::<Vec<_>>()
//             .map_ok(|_| path)
//         ).await?;

//     /* create a remote instance of ARGoS3 */
//     let process = fernbedienung::Process {
//         target: "argos3".into(),
//         working_dir: Some(software_upload_path.into()),
//         args: vec![
//             "--config".to_owned(), argos_config.to_owned(),
//             "--router".to_owned(), message_router_addr.to_string(),
//             "--id".to_owned(), uuid.to_string(),
//         ],
//     };

//     /* channel for terminating ARGoS */
//     let (terminate_tx, terminate_rx) = oneshot::channel();

//     /* create future for running ARGoS */
//     let argos_task_future = async move {
//         /* channels for routing stdout and stderr to the journal */
//         let (stdout_tx, mut stdout_rx) = mpsc::channel(8);
//         let (stderr_tx, mut stderr_rx) = mpsc::channel(8);
//         /* run argos remotely */
//         let argos = device.run(process, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
//         tokio::pin!(argos);
//         loop {
//             tokio::select! {
//                 Some(data) = stdout_rx.recv() => {
//                     let message = journal::Robot::StandardOutput(data);
//                     let event = journal::Event::Robot(uuid.to_string(), message);
//                     let request = journal::Request::Record(event);
//                     if let Err(error) = journal.send(request).await {
//                         log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
//                     }
//                 },
//                 Some(data) = stderr_rx.recv() => {
//                     let message = journal::Robot::StandardError(data);
//                     let event = journal::Event::Robot(uuid.to_string(), message);
//                     let request = journal::Request::Record(event);
//                     if let Err(error) = journal.send(request).await {
//                         log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
//                     }
//                 },
//                 exit_status = &mut argos => break exit_status,
//             }
//         }
//     };
//     Ok((argos_task_future, terminate_tx)) 
// }