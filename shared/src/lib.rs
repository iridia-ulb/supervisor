pub mod drone;
pub mod pipuck;
pub mod experiment;

use serde::{Serialize, Deserialize};
// ------ UpMsg ------

// Request should be for messages that go across an IP boundary and
// should always include a UUID
// Actions should be only used internally

// frontend to backend,
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum UpMessage {
    // perform action on drone with this id
    DroneAction(String, drone::Action),
    // perform action on pipuck with this id
    PiPuckAction(String, pipuck::Action),
    Experiment(experiment::Request)
}

// backend to frontend, status updates
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DownMessage {
    // broadcast, trigger by change in actual drone
    AddDrone(drone::Descriptor),
    UpdateDrone(String, drone::Update),
    AddPiPuck(pipuck::Descriptor),
    UpdatePiPuck(String, pipuck::Update),
    UpdateExperiment(experiment::Update),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum FernbedienungAction {
    Halt,
    Reboot,
    Bash(TerminalAction),
    SetCameraStream(bool),
    GetKernelMessages,
    Identify,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum TerminalAction {
    Start,
    Run(String),
    Stop,
}


#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentStatus {
    pub pipucks: u32,
    pub drones: u32,
}

impl Default for ExperimentStatus {
    fn default() -> Self {
        Self {
            pipucks: 0,
            drones: 0
        }
    }
}
