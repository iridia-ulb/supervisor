use std::{convert::TryInto, num::TryFromIntError, time::Duration};
use tokio::{sync::{broadcast, mpsc, oneshot}};
use futures::{Future, FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use tokio_stream;

use crate::network::{fernbedienung, fernbedienung_ext::MjpegStreamerStream, xbee};
use crate::journal;
use crate::software;

pub use shared::drone::{Action, Update};

const DRONE_BATT_FULL_MV: f32 = 4050.0;
const DRONE_BATT_EMPTY_MV: f32 = 3500.0;
const DRONE_BATT_NUM_CELLS: f32 = 3.0;
const DRONE_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[
    ("/dev/camera0", 1024, 768, 8000),
    ("/dev/camera1", 1024, 768, 8001),
    ("/dev/camera2", 1024, 768, 8002),
    ("/dev/camera3", 1024, 768, 8003),
];

pub enum Request {
    AssociateFernbedienung(fernbedienung::Device),
    AssociateXbee(xbee::Device),

    // when this message is recv, all updates are sent and then
    // updates are sent only on changes
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),



    ExperimentStart {
        software: software::Software,
        journal: mpsc::Sender<journal::Request>,
        callback: oneshot::Sender<Result<(), Error>>
    },
    ExperimentStop,
}


pub type Sender = mpsc::Sender<Request>;
pub type Receiver = mpsc::Receiver<Request>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Operation timed out")]
    Timeout,
    #[error("{0:?} is not currently valid")]
    InvalidAction(Action),

    #[error("Could not request action")]
    RequestError,
    #[error("Did not receive response")]
    ResponseError,

    #[error(transparent)]
    XbeeError(#[from] xbee::Error),
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),
    #[error(transparent)]
    FetchError(#[from] reqwest::Error),
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

enum FernbedienungRequest {
    StartCameraStream,
    StopCameraStream,
    //StartArgos,
    //StopArgos,

}

enum XbeeRequest {
    SetUpCorePower(bool),
}

fn xbee_link_margin_stream<'dev>(
    device: &'dev xbee::Device
) -> impl Stream<Item = Result<i32, String>> + 'dev {
    async_stream::stream! {
        loop {
            let strength: Result<i32, String> = device.link_margin().await
                .map_err(|error| error.to_string());
            yield strength;
        }
    }
}

async fn xbee(
    device: xbee::Device,
    mut rx: mpsc::Receiver<XbeeRequest>,
    updates_tx: broadcast::Sender<Update>
) -> anyhow::Result<()> {
    /* link strength */
    let link_strength_stream = xbee_link_margin_stream(&device)
        .map(Update::XbeeSignal);
    let link_strength_stream_throttled =
        tokio_stream::StreamExt::throttle(link_strength_stream, Duration::from_millis(1000));
    tokio::pin!(link_strength_stream_throttled);
    
    loop {
        tokio::select! {
            Some(update) = link_strength_stream_throttled.next() => {
                let _ = updates_tx.send(update);
            },
            recv = rx.recv() => match recv {
                Some(request) => match request {
                    XbeeRequest::SetUpCorePower(_) => todo!(),
                },
                None => break,
            }
        }
    }
    Ok(())
}

fn fernbedienung_link_strength_stream<'dev>(
    device: &'dev fernbedienung::Device
) -> impl Stream<Item = Result<i32, String>> + 'dev {
    async_stream::stream! {
        loop {
            let strength: Result<i32, String> = device.link_strength().await
                .map_err(|error| error.to_string());
            yield strength;
        }
    }
}

async fn fernbedienung(
    device: fernbedienung::Device,
    mut rx: mpsc::Receiver<FernbedienungRequest>,
    updates_tx: broadcast::Sender<Update>
) -> anyhow::Result<()> {
    /* link strength */
    let link_strength_stream = fernbedienung_link_strength_stream(&device)
        .map(Update::FernbedienungSignal);
    let link_strength_stream_throttled =
        tokio_stream::StreamExt::throttle(link_strength_stream, Duration::from_millis(1000));
    tokio::pin!(link_strength_stream_throttled);
    /* handle for the camera stream */
    let mut cameras_stream: tokio_stream::StreamMap<String, _> =
        tokio_stream::StreamMap::new();
    loop {
        tokio::select! {
            Some((camera, result)) = cameras_stream.next() => {
                let result: reqwest::Result<bytes::Bytes> = result;
                let update = Update::Camera { camera, result: result.map_err(|e| e.to_string()) };
                let _ = updates_tx.send(update);
            },
            Some(update) = link_strength_stream_throttled.next() => {
                let _ = updates_tx.send(update);
            },
            recv = rx.recv() => match recv {
                Some(request) => match request {
                    FernbedienungRequest::StartCameraStream => {
                        cameras_stream.clear();
                        for &(camera, width, height, port) in DRONE_CAMERAS_CONFIG {
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

pub async fn new(mut request_rx: Receiver) {
    /* fernbedienung task state */
    let fernbedienung_task = futures::future::pending().left_future();
    let mut fernbedienung_tx = Option::default();
    tokio::pin!(fernbedienung_task);
    /* xbee task state */
    let xbee_task = futures::future::pending().left_future();
    let mut xbee_tx = Option::default();
    tokio::pin!(xbee_task);
    /* updates_tx is for sending changes in state to subscribers (e.g., the webui) */
    let (updates_tx, _) = broadcast::channel(16);
    

    // TODO: for a clean shutdown we may want to consider the case where updates_tx hangs up
    loop {
        tokio::select! {
            Some(request) = request_rx.recv() => match request {
                Request::AssociateFernbedienung(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    fernbedienung_tx = Some(tx);
                    fernbedienung_task.set(fernbedienung(device, rx, updates_tx.clone()).right_future());
                },
                Request::AssociateXbee(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    xbee_tx = Some(tx);
                    xbee_task.set(xbee(device, rx, updates_tx.clone()).right_future());
                },
                Request::Subscribe(callback) => {
                    /* note that upon subscribing all updates should be sent to ensure
                       that new clients are in sync */
                    let _ = callback.send(updates_tx.subscribe());
                    // Test
                    let _ = updates_tx.send(Update::FernbedienungSignal(Ok(42)));
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
            },
            result = &mut xbee_task => {
                xbee_tx = None;
                xbee_task.set(futures::future::pending().left_future());
                if let Err(error) = result {
                    log::info!("Xbee task terminated: {}", error);
                }
            }
        }
    }
}