use serde::{Serialize, Deserialize};
use uuid::Uuid;

pub mod drone;
pub mod pipuck;
pub mod experiment;

pub mod tracking_system {
    use serde::{Serialize, Deserialize};
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Update {
        pub id: i32,
        pub position: [f32; 3],
        pub orientation: [f32; 4],
    }
}

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
    UpdateTrackingSystem(Vec<tracking_system::Update>),
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

