pub mod drone;
pub mod pipuck;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    NetworkError(#[from] crate::network::fernbedienung::Error),

    #[error(transparent)]
    PiPuckError(#[from] pipuck::Error),
}
