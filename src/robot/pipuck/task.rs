// Action, Receiver, Sender, Update, Descriptor

use crate::network::fernbedienung;
use crate::robot::FernbedienungAction;
use shared::experiment::software::Software;
use tokio::sync::{broadcast, mpsc, oneshot};

pub use shared::pipuck::{Descriptor, Update};

#[derive(Debug)]
pub enum Action {
    AssociateFernbedienung(fernbedienung::Device),
    ExecuteFernbedienungAction(oneshot::Sender<anyhow::Result<()>>, FernbedienungAction),
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),
    StartExperiment(Software),
    StopExperiment,
}

pub type Sender = mpsc::Sender<Action>;
pub type Receiver = mpsc::Receiver<Action>;

pub async fn new(rx: Receiver) {

}