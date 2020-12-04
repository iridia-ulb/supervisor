use crate::network::{ssh, xbee};
use serde::{Deserialize, Serialize};
use uuid;
use log;

const UPCORE_POWER_BIT_INDEX: u8 = 11;
const PIXHAWK_POWER_BIT_INDEX: u8 = 12;
const MUX_CONTROL_BIT_INDEX: u8 = 4;

#[derive(Debug)]
pub enum State {
    Standby,
    Ready(ssh::Device),
}

// Note: the power off, shutdown, reboot up core actions
// should change the state to standby which, in turn,
// should move the IP address back to the probing pool
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Power on UpCore")]
    UpCorePowerOn,
    #[serde(rename = "Shutdown UpCore")]
    UpCoreShutdown,
    #[serde(rename = "Power off UpCore")]
    UpCorePowerOff,
    #[serde(rename = "Reboot UpCore")]
    UpCoreReboot,
    #[serde(rename = "Power on Pixhawk")]
    PixhawkPowerOn,
    #[serde(rename = "Power off Pixhawk")]
    PixhawkPowerOff,
    #[serde(rename = "Identify")]
    Identify,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    XbeeError(#[from] xbee::Error),
    #[error(transparent)]
    SshError(#[from] ssh::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Drone {
    pub uuid: uuid::Uuid,
    pub xbee: xbee::Device,
    state: State,
}

impl Drone {
    pub fn new(xbee: xbee::Device) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4(), 
            xbee,
            state: State::Standby,
        }
    }

    pub fn ssh(&mut self) -> Option<&mut ssh::Device> {
        match &mut self.state {
            State::Standby => None,
            State::Ready(ssh) => Some(ssh),
        }
    } 

    pub async fn init(&mut self) -> Result<()> {
        /* pin configuration */
        let pin_disable_output: u8 = 0;
        let pin_digital_output: u8 = 4;
        /* mux configuration */
        let mut dio_config: u16 = 0b0000_0000_0000_0000;
        let mut dio_set: u16 = 0b0000_0000_0000_0000;
        dio_config |= 1 << MUX_CONTROL_BIT_INDEX;
        dio_set |= 1 << MUX_CONTROL_BIT_INDEX;
        /* prepare commands to be sent */
        let init_commands = vec![
            /* The UART pins need to be disabled for the moment */
            /* D7 -> CTS, D6 -> RTS, P3 -> DOUT, P4 -> DIN */
            /* disabled pins */
            xbee::Command::new("D7", &pin_disable_output.to_be_bytes()),
            xbee::Command::new("D6", &pin_disable_output.to_be_bytes()),
            xbee::Command::new("P3", &pin_disable_output.to_be_bytes()),
            xbee::Command::new("P4", &pin_disable_output.to_be_bytes()),
            /* digital output pins */
            xbee::Command::new("D4", &pin_digital_output.to_be_bytes()),
            xbee::Command::new("D1", &pin_digital_output.to_be_bytes()),
            xbee::Command::new("D2", &pin_digital_output.to_be_bytes()),
            /* mux configuration */
            xbee::Command::new("OM", &dio_config.to_be_bytes()),
            xbee::Command::new("IO", &dio_set.to_be_bytes()),
        ];
        /* send commands */
        for command in init_commands {
            self.xbee.send(command).await?;
        }
        Ok(())
    }

    /* base on the current state, create a vec of valid actions */
    pub fn actions(&self) -> Vec<Action> {
        match self.state {
            State::Standby => {
                vec![Action::UpCorePowerOn]
            },
            State::Ready{..} => {
                vec![Action::UpCorePowerOff, Action::UpCoreShutdown]
            }
        }
    }

    pub fn execute(&self, action: &Action) {
        /* check to see if the requested action is still valid */
        if self.actions().contains(&action) {
            match action {
                Action::UpCorePowerOn => {
                    log::error!("drone::Action::UpCorePowerOn is not implemented")
                },
                Action::UpCoreShutdown => {
                    log::error!("drone::Action::UpCoreShutdown is not implemented")
                },
                Action::UpCorePowerOff => {
                    log::error!("drone::Action::UpCorePowerOff is not implemented")
                },
                Action::UpCoreReboot => {
                    log::error!("drone::Action::UpCoreReboot is not implemented")
                },
                Action::PixhawkPowerOn => {
                    log::error!("drone::Action::PixhawkPowerOn is not implemented")
                },
                Action::PixhawkPowerOff => {
                    log::error!("drone::Action::PixhawkPowerOff is not implemented")
                },
                Action::Identify => {
                    log::error!("drone::Action::Identify is not implemented")
                }
            }
        }
        else {
            log::warn!("{:?} ignored due to change in drone state", action);
        }
    }

    pub async fn _set_power(&mut self, upcore: Option<bool>, pixhawk: Option<bool>) -> Result<()> {
        let mut dio_config: u16 = 0b0000_0000_0000_0000;
        let mut dio_set: u16 = 0b0000_0000_0000_0000;
        /* enable upcore power? */
        if let Some(enable_upcore_power) = upcore {
            dio_config |= 1 << UPCORE_POWER_BIT_INDEX;
            if enable_upcore_power {
                dio_set |= 1 << UPCORE_POWER_BIT_INDEX;
            }
        }
        /* enable pixhawk power? */
        if let Some(enable_pixhawk_power) = pixhawk {
            dio_config |= 1 << PIXHAWK_POWER_BIT_INDEX;
            if enable_pixhawk_power {
                dio_set |= 1 << PIXHAWK_POWER_BIT_INDEX;
            }
        }
        let cmd_om = xbee::Command::new("OM", &dio_config.to_be_bytes());
        let cmd_io = xbee::Command::new("IO", &dio_set.to_be_bytes());
        self.xbee.send(cmd_om).await?;
        self.xbee.send(cmd_io).await?;
        Ok(())
    }
}

impl super::Identifiable for Drone {
    fn id(&self) -> &uuid::Uuid {
        &self.uuid
    }

    fn set_id(&mut self, id: uuid::Uuid) {
        self.uuid = id;
    }
}
