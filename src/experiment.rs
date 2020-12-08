
use std::path::PathBuf;
use serde::{
    Deserialize,
    Serialize
};

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

#[derive(Debug)]
pub struct Experiment {
    state: State,
    pub drone_software: Software,
    pub pipuck_software: Software,
}

impl Default for Experiment {
    fn default() -> Self {
        Self {
            state: State::Stopped,
            drone_software: Software::default(),
            pipuck_software: Software::default(),
        }
    }
}

impl Experiment {
    pub fn actions(&self) -> Vec<Action> {
        match self.state {
            State::Started => vec![Action::Stop],
            State::Stopped => vec![Action::Start]
        }
    }

    pub fn execute(&mut self, action: &Action) {
        /* check to see if the requested action is still valid */
        if self.actions().contains(&action) {
            match action {
                Action::Stop => {
                    self.state = State::Stopped;
                },
                Action::Start => {
                    self.state = State::Started;
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

    // this function should check that one .argos file exists,
    // that it points to a .lua file in the same directory (relatively)
    pub fn check(&self) -> Result<(), u8> {
        Ok(())
    }

}

// pub async fn clear_ctrl_software(&mut self) -> Result<()> {
//     let path = self.shell.exec("mktemp -d").await?;
//     self.ctrl_software_path = Some(path).map(PathBuf::from);
//     Ok(())
// }

// pub async fn add_ctrl_software<F, C>(&mut self, filename: F, contents: C) -> Result<()>
//     where F: AsRef<Path>, C: AsRef<[u8]> {
//     let directory = self.ctrl_software_path.as_ref()
//         .map(PathBuf::to_owned)
//         .ok_or(Error::IoFailure)?;
//     self.upload(filename, directory, contents, 0o644).await
// }

// pub async fn ctrl_software(&mut self) -> Result<Vec<(String, String)>> {
//     if let Some(path) = &self.ctrl_software_path {
//         let query = format!("find {} -type f -exec md5sum '{{}}' \\;", path.to_string_lossy());
//         let response = self.shell.exec(query).await?
//             .split_whitespace()
//             .tuples::<(_, _)>()
//             .map(|(c, p)| (String::from(c), String::from(p)))
//             .collect::<Vec<_>>();
//         Ok(response)
//     }
//     else {
//         Ok(Vec::new())
//     }
// }