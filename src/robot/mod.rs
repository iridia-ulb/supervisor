use std::path::{Path, PathBuf};
use async_trait::async_trait;

pub mod drone;
pub mod pipuck;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("SSH is not available")]
    SshUnavailable,

    #[error(transparent)]
    SshError(#[from] crate::network::ssh::Error),
}

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

    /// installs software and returns the installation directory so that we can run argos
    async fn install(&mut self, software: &crate::software::Software) -> Result<PathBuf> {
        if let Some(ssh) = self.ssh() {
            let install_path = ssh.create_temp_dir().await?;
            for (filename, contents) in software.0.iter() {
                ssh.upload(install_path.as_path(), filename, contents, 0o644).await?;
            }
            Ok(install_path)
        }
        else {
            Err(Error::SshUnavailable)
        }
    }

    async fn start<W, C>(&mut self, _working_dir: &Path, _configuration: &Path) {
        /* start ARGoS */
    }
}