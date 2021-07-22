use moonlight::serde_lite::{self, Deserialize, Serialize};

pub mod drone;
pub mod pipuck;

// ------ UpMsg ------

// frontend to backend,
#[derive(Serialize, Deserialize, Debug)]
pub enum UpMsg {
    //send me all drones/pipucks
    Refresh,
    // perform action on drone with this id
    DroneAction(String, String)
}

// ------ DownMsg ------

// backend to frontend, status updates
#[derive(Serialize, Deserialize, Debug)]
pub enum DownMsg {
    // broadcast, trigger by change in actual drone
    UpdateDrone(String, drone::Update),
    UpdatePiPuck(String, pipuck::Update),
}

// ------ Message ------
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
