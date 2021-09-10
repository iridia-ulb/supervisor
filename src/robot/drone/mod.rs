use tokio::{self, sync::mpsc, task::JoinHandle};

mod task;
mod codec;

pub use task::{
    Action, Receiver, Sender, Update, Descriptor
};

pub struct Instance {
    pub action_tx: Sender,
    task: JoinHandle<()>
}

impl Default for Instance {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(8);
        let task = tokio::spawn(task::new(action_rx));
        Self { 
            action_tx,
            task
        }
    }
}