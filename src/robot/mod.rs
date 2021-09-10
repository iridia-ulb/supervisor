pub mod drone;
pub mod pipuck;

#[derive(Debug)]
pub enum FernbedienungAction {
    Halt,
    Reboot,
    Bash(TerminalAction),
    SetCameraStream(bool),
    UploadToTemporaryPath(Vec<(String, Vec<u8>)>),
    GetKernelMessages,
    Identify,
}

#[derive(Debug)]
pub enum XbeeAction {
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