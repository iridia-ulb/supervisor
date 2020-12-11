
use futures::{StreamExt, stream::FuturesUnordered};
use crate::software;

use serde::{
    Deserialize,
    Serialize
};

use crate::robot::{Robot, Controllable};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    Start,
    Stop,
}

#[derive(Debug)]
enum State {
    Started,
    Stopped,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not upload software: {0}")]
    UploadFailure(#[from] crate::network::ssh::Error),
    /*
    #[error("Connection timed out")]
    Timeout,
    */
}

//pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Experiment {
    state: State,
    robots: crate::Robots,
    pub drone_software: software::Software,
    pub pipuck_software: software::Software,
}

impl Experiment {
    pub fn with_robots(robots: crate::Robots) -> Self {
        Self {
            state: State::Stopped,
            robots: robots,
            drone_software: software::Software::default(),
            pipuck_software: software::Software::default(),
        }
    }

    pub fn actions(&self) -> Vec<Action> {
        match self.state {
            State::Started => vec![Action::Stop],
            State::Stopped => vec![Action::Start]
        }
    }

    pub async fn execute(&mut self, action: &Action) {
        /* check to see if the requested action is still valid */
        if self.actions().contains(&action) {
            match action {
                Action::Stop => {
                    self.state = State::Stopped;
                },
                Action::Start => {
                    /* create appropiate bindings to self */
                    let Experiment {
                        ref mut state,
                        ref robots,
                        ref drone_software,
                        ref pipuck_software
                    } = self;
                    /* define the upload tasks */
                    let mut robots = robots.write().await;
                    let mut tasks = robots.iter_mut()
                        .map(|robot| match robot {
                            Robot::PiPuck(pipuck) => pipuck.install(pipuck_software),
                            Robot::Drone(drone) => drone.install(drone_software)
                        })
                        .collect::<FuturesUnordered<_>>();
                    /* upload the software to connected robots in parallel */
                    while let Some(result) = tasks.next().await {
                        match result {
                            Ok(install_dir) => log::info!("Installed software to {}", install_dir.to_string_lossy()),
                            Err(error) => {
                                log::error!("{}", error);
                                /* if there are any errors, abort starting the experiment */
                                log::warn!("Starting the experiment aborted");
                                return;
                            }
                        }
                    }
                    /* at this point, we consider the experiment started, update the
                       state of the experiment to reflect this */
                    *state = State::Started;
                },
            }
        }
        else {
            log::warn!("{:?} ignored due to change in experiment state", action);
        }
    }
}