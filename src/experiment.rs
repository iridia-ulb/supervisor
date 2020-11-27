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
}

impl Default for Experiment {
    fn default() -> Self {
        Self {
            state: State::Stopped
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