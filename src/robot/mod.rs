pub mod drone;
pub mod pipuck;

#[derive(Debug)]
pub enum Robot {
    Drone(drone::Drone),
    PiPuck(pipuck::PiPuck),
}

pub trait Identifiable {
    fn id(&self) -> &uuid::Uuid;
    fn set_id(&mut self, id: uuid::Uuid);
}

impl Identifiable for Robot {
    fn id(&self) -> &uuid::Uuid {
        match self {
            Robot::Drone(drone) => drone.id(),
            Robot::PiPuck(pipuck) => pipuck.id(),
        }
    }

    fn set_id(&mut self, id: uuid::Uuid) {
        match self {
            Robot::Drone(drone) => drone.set_id(id),
            Robot::PiPuck(pipuck) => pipuck.set_id(id),
        };
    }
}


