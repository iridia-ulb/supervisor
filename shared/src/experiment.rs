use serde::{Serialize, Deserialize};

#[derive(Debug, Deserialize, Clone, Copy, Serialize)]
pub enum State {
    Standby,
    Active,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Request {
    AddDroneSoftware(String, Vec<u8>),
    ClearDroneSoftware,
    AddPiPuckSoftware(String, Vec<u8>),
    ClearPiPuckSoftware,
    GetExperimentState,
    StartExperiment,
    StopExperiment,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Update {
    State(State),
    DroneSoftware {
        checksums: Vec<(String, String)>,
        status: Result<(), String>
    },
    PiPuckSoftware {
        checksums: Vec<(String, String)>,
        status: Result<(), String>
    },
}