use anyhow::{Result, Context};
use futures::{Stream, StreamExt, TryFutureExt, TryStreamExt};
use shared::{builderbot, drone, pipuck};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use std::net::SocketAddr;
use std::fs::File;
use std::io::BufWriter;
use bytes::BytesMut;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use chrono::{DateTime, Local};
use shared::tracking_system;


use crate::{optitrack, router};

pub enum Action {
    Start(oneshot::Sender<anyhow::Result<()>>),
    Stop,
    Record(Event),
}

#[derive(Debug, Serialize)]
pub enum Event {
    ARGoS(String, ARGoS),
    Message(SocketAddr, crate::router::LuaType),
    TrackingSystem(Vec<tracking_system::Update>),
    Descriptors(Vec<builderbot::Descriptor>, Vec<drone::Descriptor>, Vec<pipuck::Descriptor>, )
}

#[derive(Debug, Serialize)]
pub enum ARGoS {
    StandardOutput(BytesMut),
    StandardError(BytesMut),
}

#[derive(Debug, Serialize)]
struct Entry {
    timestamp: i64,
    event: Event,
}

// ideally there would be exactly one way to subscribe to data, however, adding a subscription-style
// way of getting the data from ARGoS would require changing how the arena works since the proceedure
// for starting experiments currently prevents the arena from processing such requests

// one of the design flaws seems that there are too many layers, e.g., that the actors controlling the
// robots are stored inside the arena actor -- is there any reason why these robot actors can't just be
// in a shared hashmap that is constructed once and then passed around immutably? the send method on the
// channels doesn't need to be mutable

// the design flaw is most certainly the arena actor -- there is actually little that this actor does
// other than create an additional layer of complexity
pub async fn new(mut requests_rx: mpsc::Receiver<Action>,
                 optitrack_tx: mpsc::Sender<optitrack::Action>,
                 router_tx: mpsc::Sender<router::Action>) -> Result<()> {
    
    let optitrack_stream = futures::stream::pending().left_stream();
    tokio::pin!(optitrack_stream);
    let router_stream = futures::stream::pending().left_stream();
    tokio::pin!(router_stream);
    let mut journal: Option<(DateTime<Local>, BufWriter<_>)> = None;

    loop {
        tokio::select! {
            Some(update) = optitrack_stream.next() => match update {
                Ok(event) => {
                    let (start, writer) = journal.as_mut().unwrap();
                    let entry = Entry {
                        timestamp: Local::now()
                            .signed_duration_since(*start)
                            .num_milliseconds(),
                        event
                    };
                    if let Err(error) = serde_pickle::ser::to_writer(writer, &entry, true) {
                        log::error!("Error writing entry {:?} to journal: {}", entry, error);
                    }
                }
                Err(error) => {
                    log::error!("Error writing entries to journal: {}", error);
                }
            },
            Some(update) = router_stream.next() => match update {
                Ok(event) => if let Some((start, writer)) = journal.as_mut() {
                    let entry = Entry {
                        timestamp: Local::now()
                            .signed_duration_since(*start)
                            .num_milliseconds(),
                        event
                    };
                    if let Err(error) = serde_pickle::ser::to_writer(writer, &entry, true) {
                        log::error!("Error writing entry {:?} to journal: {}", entry, error);
                    }
                }
                Err(error) => {
                    log::error!("Error writing entries to journal: {}", error);
                }
            },
            request = requests_rx.recv() => match request {
                None => break,
                Some(action) => match action {
                    Action::Start(callback) => {
                        let now = Local::now();
                        let log_filename = now.format("%Y%m%d-%H%M%S.pkl").to_string();
                        let file_result = File::create(log_filename)
                            .context("Could not create file for journal");
                        let router_result = router(&router_tx).await;
                        let optitrack_result = optitrack(&optitrack_tx).await;
                        match (file_result, router_result, optitrack_result) {
                            (Ok(file), Ok(router), Ok(optitrack)) => {
                                journal = Some((now, BufWriter::new(file)));
                                router_stream.set(router.right_stream());
                                optitrack_stream.set(optitrack.right_stream());
                                let _ = callback.send(Ok(()));
                            },
                            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                                let _ = callback.send(Err(error));
                            }
                        }
                    },
                    Action::Stop => {
                        optitrack_stream.set(futures::stream::pending().left_stream());
                        router_stream.set(futures::stream::pending().left_stream());
                        journal = None;
                    },
                    Action::Record(event) => {
                        let (start, writer) = journal.as_mut().unwrap();
                        let entry = Entry {
                            timestamp: Local::now()
                                .signed_duration_since(*start)
                                .num_milliseconds(),
                            event
                        };
                        if let Err(error) = serde_pickle::ser::to_writer(writer, &entry, true) {
                            log::error!("Error writing entry {:?} to journal: {}", entry, error);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn router(
    router_tx: &mpsc::Sender<router::Action>
) -> anyhow::Result<impl Stream<Item = Result<Event, BroadcastStreamRecvError>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let router_updates = router_tx.send(router::Action::Subscribe(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not subscribe to router updates"))
        .and_then(move |_| callback_rx
            .map_err(|_| anyhow::anyhow!("Could not subscribe to router updates")));
    router_updates.await
        .map(|updates| BroadcastStream::new(updates)
            .map_ok(|(socket, message)| Event::Message(socket, message)))
}

async fn optitrack(
    optitrack_tx: &mpsc::Sender<optitrack::Action>
) -> anyhow::Result<impl Stream<Item = Result<Event, BroadcastStreamRecvError>>> {
    let (callback_tx, callback_rx) = oneshot::channel();
    let optitrack_updates = optitrack_tx.send(optitrack::Action::Subscribe(callback_tx))
        .map_err(|_| anyhow::anyhow!("Could not subscribe to tracking system updates"))
        .and_then(move |_| callback_rx
            .map_err(|_| anyhow::anyhow!("Could not subscribe to tracking system updates")));
    optitrack_updates.await
        .map(|updates| BroadcastStream::new(updates)
            .map_ok(Event::TrackingSystem))
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