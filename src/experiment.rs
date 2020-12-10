
use std::path::PathBuf;
use futures::{StreamExt, stream::FuturesUnordered};

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
    pub drone_software: Software,
    pub pipuck_software: Software,
}

impl Experiment {
    pub fn with_robots(robots: crate::Robots) -> Self {
        Self {
            state: State::Stopped,
            robots: robots,
            drone_software: Software::default(),
            pipuck_software: Software::default(),
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
                            Ok(install_dir) => log::info!("installed software to {}", install_dir.to_string_lossy()),
                            Err(error) => {
                                log::error!("{}", error);
                                /* if there are any errors, abort starting the experiment */
                                log::warn!("starting the experiment aborted");
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


// the idea here is that we have a single instance of the software for all
// drones and a single instance for all pipucks.
// two instances of this may live inside the experiment struct, do we even need the firmware module?
// as part of starting an experiment, this content is downloaded
#[derive(Default, Debug)]
pub struct Software(pub Vec<(PathBuf, Vec<u8>)>);

impl Software {
    pub fn add<F: Into<PathBuf>, C: Into<Vec<u8>>>(&mut self, new_filename: F, new_contents: C) {
        let new_filename = new_filename.into();
        let new_contents = new_contents.into();
        if let Some((_, contents)) = self.0.iter_mut()
            .find(|(filename, _)| filename == &new_filename) {
            contents.splice(.., new_contents.into_iter());
        }
        else {
            self.0.push((new_filename, new_contents));
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    // TODO move software into firmware.rs and refactor
    pub fn argos_config(&self) -> Result<&(PathBuf, Vec<u8>), String> {
        // also count! there should be only 1
        match self.0.iter().find(|entry| entry.0.to_string_lossy().contains(".argos")) {
            Some(file) => Ok(file),
            None => Err("nup".to_owned())
        }
    } 
   
    pub fn check_config(&self) -> Result<(), String> {


        Ok(())
    }
}
