use serde::{Serialize, Deserialize};
use uuid::Uuid;

pub mod drone;
pub mod pipuck;
pub mod experiment;

// backend to frontend
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DownMessage {
    Request(Uuid, FrontEndRequest),
    Response(Uuid, Result<(), String>), // response to a up message
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum FrontEndRequest {
    AddDrone(drone::Descriptor),
    UpdateDrone(String, drone::Update),
    AddPiPuck(pipuck::Descriptor),
    UpdatePiPuck(String, pipuck::Update),
    UpdateExperiment(experiment::Update),
}

// frontend to backend
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum UpMessage {
    Request(Uuid, BackEndRequest),
    Response(Uuid, Result<(), String>), // response to a down message
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum BackEndRequest {
    DroneRequest(String, drone::Request),
    PiPuckRequest(String, pipuck::Request),
    ExperimentRequest(experiment::Request),
}

