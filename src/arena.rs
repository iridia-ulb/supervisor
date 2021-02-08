
use futures::{StreamExt, stream::FuturesUnordered};
use log;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::robot::{pipuck::{self, PiPuck}, drone::{self, Drone}};
/*
arena add drone/pipuck ...
arena set drone 

*/

// todo add RequestKind (enum), Request becomes (uuid, action, callback (oneshot<result>))

pub enum Request {
    AddDrone(Drone),
    AddPiPuck(PiPuck),
    // GetDroneActions(Vec<drone::Action>),
    // ExecuteDroneAction(drone::Action),
    GetPiPuckActions(Uuid, Vec<pipuck::Action>),
    ExecutePiPuckAction(Uuid, pipuck::Action, oneshot::Sender<pipuck::Result<()>>),
}

pub async fn new(mut arena_request_rx: mpsc::UnboundedReceiver<Request>) {
    let mut drones : FuturesUnordered<Drone> = Default::default();
    let mut pipucks : FuturesUnordered<PiPuck> = Default::default();

    // if let Some(request) = arena_request_rx.next().await {
    //     match request {
    //         Request::AddRobot(robot) => robots.push(robot)
    //     }
    // }

    // if let Some(result) = robots.next().await {
    //     match result {
    //         Ok(()) => log::info!("Robot task completed sucessfully"),
    //         Err(error) => log::error!("Robot task failed: {}", error),
    //     }
    // }

    tokio::select! {
        Some(request) = arena_request_rx.next() => match request {
            Request::AddDrone(drone) => drones.push(drone),
            Request::AddPiPuck(pipuck) => pipucks.push(pipuck),
            Request::ExecutePiPuckAction(uuid, action, result) => {
                
            }
        },
        Some(result) = drones.next() => match result {
            // todo: return uuid, ip of robot?
            // return unused ip address to pool?
            Ok(_) => log::info!("Robot task completed sucessfully"),
            Err(error) => log::error!("Robot task failed: {}", error),
        },
        Some(result) = pipucks.next() => match result {
            // todo: return uuid, ip of robot?
            // return unused ip address to pool?
            Ok(_) => log::info!("Robot task completed sucessfully"),
            Err(error) => log::error!("Robot task failed: {}", error),
        },
        else => {
            // I believe this only occurs when all robot tasks are complete
            // and all instances of arena_request_tx have been dropped
            log::warn!("todo! shutdown all robots? Nope, not here until robots have already shutdown")
        }
    }
}

async fn execute_pipuck_action(pipucks) {

}