use serde::{Serialize, Deserialize};
pub mod software;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Request {
    Start {
        drone_software: software::Software,
        pipuck_software: software::Software,
    },
    Stop,
}

#[derive(Debug, Deserialize, Clone, Copy, Serialize)]
pub enum State {
    Standby,
    Active,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Update {
    State(State),
}