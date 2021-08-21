use tokio::{self, sync::mpsc, task::JoinHandle};

mod task;
mod codec; // TODO move this inside task?

pub use task::{
    Action, Error, Receiver, Request, Sender, Update, Descriptor
};

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