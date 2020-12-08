use std::path::PathBuf;
use async_trait::async_trait;

use crate::network::ssh::Result;

pub mod drone;
pub mod pipuck;

#[derive(Debug)]
pub enum Robot {
    Drone(drone::Drone),
    PiPuck(pipuck::PiPuck),
}

pub trait Identifiable {
    fn id(&self) -> &uuid::Uuid;
    fn set_id(&mut self, id: uuid::Uuid);
}

impl Identifiable for Robot {
    fn id(&self) -> &uuid::Uuid {
        match self {
            Robot::Drone(drone) => drone.id(),
            Robot::PiPuck(pipuck) => pipuck.id(),
        }
    }

    fn set_id(&mut self, id: uuid::Uuid) {
        match self {
            Robot::Drone(drone) => drone.set_id(id),
            Robot::PiPuck(pipuck) => pipuck.set_id(id),
        };
    }
}

// the drone and pipuck will both need to have a field for its control software
// perhaps even a field that represents an instance of ARGoS
#[async_trait]
pub trait Controllable {
    // this method returns None if ssh is not available (device state!?)
    // and Some(ssh) is the device is available
    // it is not clear how the device state fits into this picture yet
    // it may be correct to ignore device state, and all other methods in this
    // trait be failable (which they already are)
    fn ssh(&mut self) -> Option<&mut crate::network::ssh::Device>;

    /// path to the control software
    fn path(&self) -> PathBuf;

    async fn upload(&mut self, software: &crate::experiment::Software) -> crate::network::ssh::Result<()> {
        let ref upload_path = self.path();
        if let Some(ssh) = self.ssh() {
            for (filename, contents) in software.0.iter() {
                ssh.upload(filename, upload_path, contents, 0o644).await?;
            }
        }
        Ok(())
    }

    async fn start(&mut self); // start ARGoS?
}