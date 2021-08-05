pub mod drone;
pub mod pipuck;

// ------ UpMsg ------

// frontend to backend,
//#[derive(Serialize, Deserialize, Debug)]
pub enum UpMessage {
    //send me all drones/pipucks
    Refresh,
    // perform action on drone with this id
    DroneAction(String, String)
}

// backend to frontend, status updates
//#[derive(Serialize, Deserialize, Debug)]
pub enum DownMessage {
    // broadcast, trigger by change in actual drone
    UpdateDrone(String, drone::Update),
    UpdatePiPuck(String, pipuck::Update),
}


//#[derive(Serialize, Deserialize, Clone, Debug)]
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
