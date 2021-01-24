use crate::network::fernbedienung;
use serde::{Deserialize, Serialize};
use uuid;
use log;

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
    pub device: fernbedienung::Device,
}

impl PiPuck {
    
    pub fn new(device: fernbedienung::Device) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4(), 
            device,
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
                    if let Err(error) = self.device.shutdown().await {
                        log::error!("{:?} failed with: {}", action, error);
                    }
                },
                Action::RpiReboot => {
                    if let Err(error) = self.device.reboot().await {
                        log::error!("{:?} failed with: {}", action, error);
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

impl super::Controllable for PiPuck {
    fn fernbedienung(&mut self) -> Option<&mut fernbedienung::Device> {
        Some(&mut self.device)
    }
}