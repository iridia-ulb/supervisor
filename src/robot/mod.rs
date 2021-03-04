pub mod drone;
pub mod pipuck;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
   
    #[error("Network is not available")]
    NetworkUnavailable,

    #[error(transparent)]
    NetworkError(#[from] crate::network::fernbedienung::Error),

    #[error(transparent)]
    PiPuckError(#[from] pipuck::Error),

    #[error(transparent)]
    DroneError(#[from] drone::Error),
}
