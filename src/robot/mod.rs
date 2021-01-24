use std::path::{Path, PathBuf};
use async_trait::async_trait;

pub mod drone;
pub mod pipuck;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not convert path to UTF-8")]
    InvalidPath,
    
    #[error("Network is not available")]
    NetworkUnavailable,

    #[error(transparent)]
    NetworkError(#[from] crate::network::fernbedienung::Error),  
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

/* this trait is probably doing too much */
// it would be better if this trait was split into traits for handling
#[async_trait]
pub trait Controllable {
    // this method returns None if ssh is not available (device state!?)
    // and Some(ssh) is the device is available
    // it is not clear how the device state fits into this picture yet
    // it may be correct to ignore device state, and all other methods in this
    // trait be failable (which they already are)
    fn fernbedienung(&mut self) -> Option<&mut crate::network::fernbedienung::Device>;

    /// installs software and returns the installation directory so that we can run argos
    async fn install(&mut self, software: &crate::software::Software) -> Result<PathBuf> {
        let fernbedienung = self.fernbedienung().ok_or(Error::NetworkUnavailable)?;
        let controller_path = fernbedienung.create_temp_dir().await?;
        for (filename, contents) in software.0.iter() {
            fernbedienung.upload(controller_path.as_path(), filename, contents.to_owned(), 0o644).await?;
        }
        Ok(controller_path)
    }

    // configuration is just the path to the .argos, we cd into this directory and run ARGoS in there
    async fn start<W, C>(&mut self, working_dir: W, config_file: C) -> Result<String>
        where C: AsRef<Path> + Send, W: Into<PathBuf> + Send {
        /* prepare arguments */
        let target = PathBuf::from("argos3");
        let argument = format!("-c {}", config_file.as_ref().to_string_lossy());
        /* execute */
        let fernbedienung = self.fernbedienung().ok_or(Error::NetworkUnavailable)?;
        fernbedienung.run(target, working_dir, vec![argument]).await
            .map_err(|e| Error::NetworkError(e))
    }
}