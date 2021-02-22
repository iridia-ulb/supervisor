use std::{net::SocketAddr, time::{Instant, Duration}};
use std::fs::File;
use std::io::{BufWriter, Write};
use bytes::{Bytes, BytesMut};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use std::time::{SystemTime, SystemTimeError};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    SystemTimeError(#[from] SystemTimeError),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Could not send request")]
    RequestError,
    #[error("Did not receive response")]
    ResponseError,
}

type Result<T> = std::result::Result<T, Error>;

pub enum Request {
    Start(oneshot::Sender<Result<()>>),
    Stop,
    Record(Event),
}

#[derive(Debug, Serialize)]
pub enum Event {
    Optitrack {},
    Robot(Uuid, Robot),
    Broadcast(SocketAddr, crate::router::LuaType),
}

#[derive(Debug, Serialize)]
pub enum Robot {
    StandardOutput(BytesMut),
    StandardError(BytesMut),
}



#[derive(Debug, Serialize)]
struct Entry {
    timestamp: Duration,
    event: Event,
}

// todo spawn a logging task here and return a channel for logging messages
pub async fn new(rx: mpsc::UnboundedReceiver<Request>) -> Result<()> {
    let mut requests = UnboundedReceiverStream::new(rx);
    let mut start: Option<Instant> = None;
    let mut writer: Option<BufWriter<_>> = None;
    while let Some(request) = requests.next().await {
        match request {
            // TODO add a callback from here to abort starting the experiment if the log file isn't good
            Request::Start(callback) => {
                let response = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                    Err(error) => Err(Error::SystemTimeError(error)),
                    Ok(since_unix_epoch) => {
                        let log_filename = format!("{}.pkl", since_unix_epoch.as_secs());
                        start.get_or_insert_with(Instant::now);
                        match File::create(log_filename) {
                            Err(error) => Err(Error::IoError(error)),
                            Ok(file) => {
                                writer.get_or_insert(BufWriter::new(file));
                                Ok(())
                            }
                        }
                    }
                };
                if let Err(_) = callback.send(response) {
                    log::error!("Could not respond to start experiment request");
                }
            },
            Request::Stop => {
                /* clear the start time and close the file */
                start.take();
                writer.take();
            },
            Request::Record(event) => if let Some(start) = start.as_ref() {
                if let Some(writer) = writer.as_mut() {
                    let entry = Entry { timestamp: start.elapsed(), event };
                    if let Err(error) = serde_pickle::ser::to_writer(writer, &entry, true) {
                        log::error!("Error writing entry {:?} to journal: {}", entry, error);
                    }
                }
            }
        }
    }
    Ok(())
}

/* .bashrc
depickle() {
python << EOPYTHON
import pickle
f = open('${1}', 'rb')
while True:
   try:
      print(pickle.load(f))
   except EOFError:
      break
EOPYTHON
}
*/