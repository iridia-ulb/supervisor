pub mod drone;
pub mod pipuck;

use serde::{Serialize, Deserialize};
// ------ UpMsg ------

// frontend to backend,
#[derive(Serialize, Deserialize, Debug)]
pub enum UpMessage {
    // perform action on drone with this id
    DroneAction(String, drone::Action),
    // perform action on pipuck with this id
    PiPuckAction(String, pipuck::Action),
}

// backend to frontend, status updates
#[derive(Serialize, Deserialize, Debug)]
pub enum DownMessage {
    // broadcast, trigger by change in actual drone
    AddDrone(drone::Descriptor),
    UpdateDrone(String, drone::Update),
    AddPiPuck(pipuck::Descriptor),
    UpdatePiPuck(String, pipuck::Update),
}

#[derive(Debug, Deserialize, Serialize)]
pub enum FernbedienungAction {
    Halt,
    Reboot,
    Bash(TerminalAction),
    SetCameraStream(bool),
    GetKernelMessages,
    Identify,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum TerminalAction {
    Start,
    Run(String),
    Stop,
}


#[derive(Serialize, Deserialize, Clone, Debug)]
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
