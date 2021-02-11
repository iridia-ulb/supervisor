use std::{net::Ipv4Addr, time::Duration};
use std::path::PathBuf;
//use protocol::Request;
//use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use futures::channel::{mpsc, oneshot};

use tokio::{sync::Mutex, net::TcpStream};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_serde::{SymmetricallyFramed, formats::SymmetricalJson};

mod protocol;

use protocol::{Response, ResponseKind, Request, RequestKind, Upload, process};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    // #[error(transparent)]
    // SinkError(#[from] futures::sink::Sink::Error),
    
    #[error("Remote error: {0}")]
    RemoteError(String),

    #[error("Remote operation timed out")]
    Timeout,

    #[error("Could not send request")]
    SendError,


}

pub type Result<T> = std::result::Result<T, Error>;

type Requests = SymmetricallyFramed<
    FramedWrite<tokio::io::WriteHalf<TcpStream>, LengthDelimitedCodec>,
    Request,
    SymmetricalJson<Request>>;
/*
type Responses = SymmetricallyFramed<
    FramedRead<tokio::io::ReadHalf<TcpStream>, LengthDelimitedCodec>,
    Response,
    SymmetricalJson<Response>>;
*/

type Callbacks = Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<ResponseKind>>>>;

pub struct Device {
    pub addr: Ipv4Addr,
    pub requests: Requests,
    pub callbacks: Callbacks,
    drop_tx: Option<oneshot::Sender<()>>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.addr)
         .finish()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(drop_tx) = self.drop_tx.take() {
            if let Err(_) = drop_tx.send(()) {
                log::warn!("dropped without killing loop")
            }
        }
    }
}

impl Device {
    pub async fn new(addr: Ipv4Addr) -> Result<Device> {
        let stream = TcpStream::connect((addr, 17653)).await
            .map_err(|error| Error::IoError(error))?;
        let (read, write) = tokio::io::split(stream);
        let requests = SymmetricallyFramed::new(
            FramedWrite::new(write, LengthDelimitedCodec::new()),
            SymmetricalJson::<Request>::default(),
        );
        let responses = SymmetricallyFramed::new(
            FramedRead::new(read, LengthDelimitedCodec::new()),
            SymmetricalJson::<Response>::default(),
        );
        /* create a channel for terminating */
        let (drop_tx, drop_rx) = futures::channel::oneshot::channel::<()>();

        let callbacks = Callbacks::default();
        
        let device = Device { addr, requests, drop_tx: Some(drop_tx), callbacks: callbacks.clone()};

        // to look into, perhaps there is somewhere better to run this
        tokio::spawn(async move {
            let mut drop_rx = drop_rx;
            let mut responses = responses;

            loop {
                tokio::select! {
                    _ = &mut drop_rx => break,
                    Some(response) = responses.next() => {
                        match response {
                            Ok(Response(uuid, response)) => {
                                if let Some(uuid) = uuid {
                                    if let Some(rx) = callbacks.lock().await.get_mut(&uuid) {
                                        if let Err(error)= rx.send(response).await {
                                            log::error!("could not forward message: {}", error);
                                        }
                                    }
                                    // else {
                                    //     log::warn!("no call back registered for uuid: {}", uuid);
                                    // }
                                }
                                else {
                                    log::warn!("received message without uuid: {:?}", response);
                                }
                            },
                            Err(error) => {
                                log::error!("{}", error.to_string());
                            }
                        }
                    }
                }
            }
        });
        Ok(device)
    }

    pub async fn upload<P, F, C>(&mut self, path: P, filename: F, contents: C) -> Result<()>
    where P: Into<PathBuf>, F: Into<PathBuf>, C: Into<Vec<u8>> {
        let uuid = Uuid::new_v4();
        let request = Request(uuid, RequestKind::Upload(Upload {
            filename: filename.into(),
            path: path.into(),
            contents: contents.into()
        }));
        let (tx, mut rx) = mpsc::unbounded::<ResponseKind>();
        self.callbacks.lock().await.insert(uuid, tx);
        self.requests.send(request).await.map_err(|_| Error::SendError)?;
        let result = match tokio::time::timeout(Duration::from_millis(500), rx.next()).await {
            Err(_) => Err(Error::Timeout),
            Ok(response) => match response {
                Some(response) => match response {
                    ResponseKind::Ok => Ok(()),
                    ResponseKind::Error(error) => Err(Error::RemoteError(error)),
                    _ => Err(Error::RemoteError("Invalid response".to_owned())),
                }
                None => Err(Error::RemoteError("Disconnected".to_owned())),
            }
        };
        self.callbacks.lock().await.remove(&uuid);
        result
    }

    pub async fn create() -> impl std::future::Future<Output = ()> {
        
        async {

        }
    }

    pub async fn run<T, W, A>(&mut self, target: T, working_dir: W, args: A) -> Result<()> 
        where T: Into<PathBuf>, W: Into<PathBuf>, A: Into<Vec<String>> {
        let uuid = Uuid::new_v4();

        let request = Request(uuid, RequestKind::Process(
            process::Request::Run(process::Run {
                target: target.into(),
                working_dir: working_dir.into(),
                args: args.into()
            })
        ));

        // spawn a thread? move in tx end, when the rx end is dropped or terminate is received,
        // break the loop and remove process from map?

        // investigate possibility of having a futuresunordered poll all processes?

        let (tx, rx) = mpsc::unbounded::<ResponseKind>();
        self.callbacks.lock().await.insert(uuid, tx);

        let result = match self.requests.send(request).await {
            Ok(()) => Ok(()),
            Err(_) => Err(Error::SendError),
        };
        self.callbacks.lock().await.remove(&uuid);
        result
    }

    pub async fn create_temp_dir(&mut self) -> Result<PathBuf> {
        let result = Device::run(self, "mktmp", "/tmp", vec!["-d".to_owned()]).await;
                
        Ok("/".into())
    }

    // pub async fn hostname(&self) -> Result<String> {
    //     Ok("Bob".to_owned())
    // }

    // pub async fn shutdown(&self) -> Result<()> {
    //     Ok(())
    // }

    // pub async fn reboot(&self) -> Result<()> {
    //     Ok(())
    // }

    
}





