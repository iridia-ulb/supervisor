use std::{net::Ipv4Addr, pin::Pin, future::Future};
use std::path::PathBuf;

//use protocol::Request;
//use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use bytes::BytesMut;
use mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use uuid::Uuid;
use futures::{self, FutureExt, SinkExt, StreamExt};


use tokio::{sync::Mutex, net::TcpStream};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tokio_serde::{SymmetricallyFramed, formats::SymmetricalJson};

mod protocol;

pub use protocol::{Response, ResponseKind, Request, RequestKind, Upload, process};

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

    #[error("Process stream terminated")]
    TerminationError,

    #[error("Could not convert data to UTF-8")]
    ConversionError,


}

pub type Result<T> = std::result::Result<T, Error>;

type Responses = SymmetricallyFramed<
    FramedRead<tokio::io::ReadHalf<TcpStream>, LengthDelimitedCodec>,
    Response,
    SymmetricalJson<Response>>;

pub type Requests = SymmetricallyFramed<
    FramedWrite<tokio::io::WriteHalf<TcpStream>, LengthDelimitedCodec>,
    Request,
    SymmetricalJson<Request>>;
    

pub type Callbacks = HashMap<Uuid, mpsc::UnboundedSender<ResponseKind>>;

pub struct Interface(Mutex<Requests>, Mutex<Callbacks>);

impl Interface {
    pub async fn upload(self: Arc<Self>, path: PathBuf, filename: PathBuf, contents: Vec<u8>) -> Result<()> {
        let uuid = Uuid::new_v4();
        let request = Request(uuid, RequestKind::Upload(Upload {
            filename, path, contents,
        }));

        let (tx, rx) = mpsc::unbounded_channel::<ResponseKind>();
        self.1.lock().await.insert(uuid, tx);
        self.0.lock().await.send(request).await.map_err(|_| Error::SendError)?;

        let mut response = UnboundedReceiverStream::new(rx);

        let result = match response.next().await {
            Some(response) => match response {
                ResponseKind::Ok => Ok(()),
                ResponseKind::Error(error) => Err(Error::RemoteError(error)),
                _ => Err(Error::RemoteError("Invalid response".to_owned())),
            }
            None => Err(Error::RemoteError("Disconnected".to_owned())),
        };
        self.1.lock().await.remove(&uuid);
        result
    }

    // the problem here is holding a &mut to self while this future is completing, is not
    // not necessary and would actually prevent response_task from making progress since
    // it needs Pin<&mut Self> for poll

    // regardless of what I do in this function, &mut self will be held until the future
    // completes, which means I can not take &mut self (or &self) as an argument since this
    // will prevent the future from making progress

    // this appears to be a common challenge with embedding a future in a struct
    //

    pub async fn run(self: Arc<Self>,
                     task: process::Run,
                     signal_rx: Option<UnboundedReceiver<u32>>,
                     stdin_rx: Option<UnboundedReceiver<BytesMut>>,
                     stdout_tx: Option<UnboundedSender<BytesMut>>,
                     stderr_tx: Option<UnboundedSender<BytesMut>>) -> Result<bool> {
        let uuid = Uuid::new_v4();
        let request = Request(uuid, RequestKind::Process(
            process::Request::Run(task))
        );

        /* insert callback */
        let (tx, rx) = mpsc::unbounded_channel::<ResponseKind>();
        self.1.lock().await.insert(uuid, tx);
        
        /* send request */
        self.0.lock().await
            .send(request).await
            .map_err(|_| Error::SendError)?;

        /* process response */
        let mut responses = UnboundedReceiverStream::new(rx);

        let mut signal_rx = match signal_rx {
            Some(signal_rx) => UnboundedReceiverStream::new(signal_rx).left_stream(),
            None => futures::stream::pending().right_stream(),
        };

        let mut stdin_rx = match stdin_rx {
            Some(stdin_rx) => UnboundedReceiverStream::new(stdin_rx).left_stream(),
            None => futures::stream::pending().right_stream(),
        };
        
        loop {
            tokio::select! {
                Some(signal) = signal_rx.next() => {
                    let request = Request(uuid, RequestKind::Process(
                        process::Request::Signal(signal))
                    );
                    self.0.lock().await
                        .send(request).await
                        .map_err(|_| Error::SendError)?;
                },
                Some(stdin) = stdin_rx.next() => {
                    let request = Request(uuid, RequestKind::Process(
                        process::Request::StandardInput(stdin))
                    );
                    self.0.lock().await
                        .send(request).await
                        .map_err(|_| Error::SendError)?;

                },
                Some(response) = responses.next() => match response {
                    ResponseKind::Process(response) => match response {
                        process::Response::Started => {},
                        process::Response::Terminated(result) => {
                            self.1.lock().await.remove(&uuid);
                            return Ok(result);
                        },
                        process::Response::StandardOutput(data) => if let Some(stdout_tx) = &stdout_tx {
                            if let Err(error) = stdout_tx.send(data) {
                                log::error!("Could not forward standard output to channel: {}", error);
                            }
                        },
                        process::Response::StandardError(data) => if let Some(stderr_tx) = &stderr_tx {
                            if let Err(error) = stderr_tx.send(data) {
                                log::error!("Could not forward standard error to channel: {}", error);
                            }
                        },
                    },
                    _ => {}
                },
                else => break
            }
        }
        Err(Error::TerminationError)
    }

    pub async fn create_temp_dir(self: Arc<Self>) -> Result<String> {
        let task = process::Run {
            target: "mktemp".into(),
            working_dir: "/tmp".into(),
            args: vec!["-d".to_owned()],
        };
        let (stdout_tx, stdout_rx) = mpsc::unbounded_channel();
        self.run(task, None, None, Some(stdout_tx), None).await?;
        let stdout_stream = UnboundedReceiverStream::new(stdout_rx);
        let stdout = stdout_stream.concat().await;
        let temp_dir = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::ConversionError)?;
        Ok(temp_dir.trim().to_owned())
    }

    pub async fn hostname(self: Arc<Self>) -> Result<String> {
        let task = process::Run {
            target: "hostname".into(),
            working_dir: "/tmp".into(),
            args: vec![],
        };
        let (stdout_tx, stdout_rx) = mpsc::unbounded_channel();
        self.run(task, None, None, Some(stdout_tx), None).await?;
        let stdout_stream = UnboundedReceiverStream::new(stdout_rx);
        let stdout = stdout_stream.concat().await;
        let hostname = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::ConversionError)?;
        Ok(hostname.trim().to_owned())
    }

    pub async fn shutdown(self: Arc<Self>) -> Result<bool> {
        let task = process::Run {
            target: "echo".into(),
            working_dir: "/tmp".into(),
            args: vec!["shutdown".to_owned()],
        };
        self.run(task, None, None, None, None).await
    }

    pub async fn reboot(self: Arc<Self>) -> Result<bool> {
        let task = process::Run {
            target: "echo".into(),
            working_dir: "/tmp".into(),
            args: vec!["reboot".to_owned()],
        };
        self.run(task, None, None, None, None).await
    }
}

pub type Task = Pin<Box<dyn Future<Output = ()> + Send>>;

pub struct Device(Task, Arc<Interface>, Ipv4Addr);

impl Device {
    pub async fn new(addr: Ipv4Addr) -> Result<Self> {
        let stream = TcpStream::connect((addr, 17653)).await
            .map_err(|error| Error::IoError(error))?;
        let (read, write) = tokio::io::split(stream);
        let requests: Requests = SymmetricallyFramed::new(
            FramedWrite::new(write, LengthDelimitedCodec::new()),
            SymmetricalJson::<Request>::default(),
        );
        let responses : Responses = SymmetricallyFramed::new(
            FramedRead::new(read, LengthDelimitedCodec::new()),
            SymmetricalJson::<Response>::default(),
        );
        let callbacks : Callbacks = Default::default();
        let interface = Arc::new(Interface(Mutex::new(requests), Mutex::new(callbacks)));
        let task = Device::task(responses, interface.clone()).boxed();
        Ok(Device(task, interface, addr))
    }

    pub fn split(self) -> (Task, Arc<Interface>, Ipv4Addr) {
        (self.0, self.1, self.2)
    }

    pub fn unite(task: Task, interface: Arc<Interface>, addr: Ipv4Addr) -> Self {
        Self(task, interface, addr)
    }

    async fn task(mut responses: Responses, interface: Arc<Interface>) {
        while let Some(response) = responses.next().await {
            match response {
                Ok(Response(uuid, response)) => {
                    if let Some(uuid) = uuid {
                        if let Some(tx) = interface.1.lock().await.get_mut(&uuid) {
                            if let Err(error)= tx.send(response) {
                                log::error!("could not forward message: {}", error);
                            }
                        }
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

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.2)
         .finish()
    }
}