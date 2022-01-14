pub mod builderbot;
pub mod drone;
pub mod pipuck;

use shared::experiment::software::Software;
use tokio::sync::mpsc;
use crate::journal;

#[derive(Debug)]
pub enum FernbedienungAction {
    Halt,
    Reboot,
    Bash(TerminalAction),
    SetCameraStream(bool),
    SetupExperiment(String, Software, mpsc::Sender<journal::Action>),
    StartExperiment,
    StopExperiment,
    Identify,
}

#[derive(Debug)]
pub enum XbeeAction {
    SetAutonomousMode(bool),
    SetUpCorePower(bool),
    SetPixhawkPower(bool),
    Mavlink(TerminalAction),
}

#[derive(Debug)]
pub enum TerminalAction {
    Start,
    Run(String),
    Stop,
}