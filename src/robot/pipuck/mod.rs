mod task;

pub use task::{
    Error, Receiver, Request, Sender, Update, Descriptor
};

use tokio::{sync::mpsc, task::JoinHandle};

pub struct Instance {
    pub request_tx: mpsc::Sender<Request>,
    task: JoinHandle<()>
}

impl Instance {
    pub fn new(descriptor: Descriptor) -> Self {
        let (request_tx, request_rx) = mpsc::channel(8);
        let task = tokio::spawn(task::new(request_rx, descriptor));
        Self {
            request_tx,
            task
        }
    }
}
