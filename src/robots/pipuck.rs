use super::ssh;
use serde::{Deserialize, Serialize};
use uuid;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    SshError(#[from] ssh::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Shutdown RPi")]
    RpiShutdown,
    #[serde(rename = "Reboot RPi")]
    RpiReboot,
}

#[derive(Debug)]
pub struct PiPuck {
    uuid: uuid::Uuid,
    ssh: ssh::Device,
}

impl PiPuck {
    pub fn new(ssh: ssh::Device) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4(), 
            ssh,
        }
    }

    fn actions(&self) -> Vec<Action> {
        vec![Action::RpiShutdown, Action::RpiReboot]
    }
}