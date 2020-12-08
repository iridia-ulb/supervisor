use crate::network::ssh;
use serde::{Deserialize, Serialize};
use uuid;
use log;

/*
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    SshError(#[from] ssh::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
*/

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Shutdown RPi")]
    RpiShutdown,
    #[serde(rename = "Reboot RPi")]
    RpiReboot,
    #[serde(rename = "Identify")]
    Identify,
}

#[derive(Debug)]
pub struct PiPuck {
    pub uuid: uuid::Uuid,
    pub ssh: ssh::Device,
}

impl PiPuck {
    
    pub fn new(ssh: ssh::Device) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4(), 
            ssh,
        }
    }

    pub fn actions(&self) -> Vec<Action> {
        vec![Action::RpiShutdown, Action::RpiReboot, Action::Identify]
    }

    pub async fn execute(&mut self, action: &Action) {
        /* check to see if the requested action is still valid */
        if self.actions().contains(&action) {
            match action {
                Action::RpiShutdown => {
                    match self.ssh.default_shell() {
                        Err(error) => log::error!("{:?} failed with: {}", action, error),
                        Ok(shell) => match shell.exec("shutdown 0; exit").await {
                            Ok(_) => self.ssh.disconnect(),
                            Err(error) => log::error!("{:?} failed with: {}", action, error),
                        }
                    }
                },
                Action::RpiReboot => {
                    match self.ssh.default_shell() {
                        Err(error) => log::error!("{:?} failed with: {}", action, error),
                        Ok(shell) => match shell.exec("reboot; exit").await {
                            Ok(_) => self.ssh.disconnect(),
                            Err(error) => log::error!("{:?} failed with: {}", action, error),
                        }
                    }
                },
                Action::Identify => {
                    log::error!("pipuck::Action::Identify is not implemented")
                }
            }
        }
        else {
            log::warn!("{:?} ignored due to change in Pi-Puck state", action);
        }
    }
}

impl super::Identifiable for PiPuck {
    fn id(&self) -> &uuid::Uuid {
        &self.uuid
    }

    fn set_id(&mut self, id: uuid::Uuid) {
        self.uuid = id;
    }
}