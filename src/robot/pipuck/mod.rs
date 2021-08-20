use macaddr::MacAddr6;
use tokio::{sync::mpsc, task::JoinHandle};

mod task;

pub use task::{
    Error, Receiver, Request, Sender, Update
};

#[derive(Debug)]
pub struct Descriptor {
    pub id: String,
    pub rpi_macaddr: MacAddr6,
    pub optitrack_id: Option<i32>,
    pub apriltag_id: Option<u8>,
}

pub struct Instance {
    pub descriptor: Descriptor,
    pub request_tx: mpsc::Sender<Request>,
    task: JoinHandle<()>
}

impl Instance {
    pub fn new(descriptor: Descriptor) -> Self {
        let (request_tx, request_rx) = mpsc::channel(8);
        let task = tokio::spawn(task::new(request_rx));
        Self {
            descriptor,
            request_tx,
            task
        }
    }
}
