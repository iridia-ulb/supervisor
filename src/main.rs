use bytes::{BytesMut, BufMut};
use std::time::{Duration, SystemTime};
use futures::{FutureExt, StreamExt};
use tokio::{sync::mpsc, sync::RwLock, time::delay_for}; //RwLock
use warp::ws::{Message, WebSocket};
use warp::Filter;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, collections::HashMap, net::Ipv4Addr};

#[derive(Serialize, Deserialize, Debug)]
enum PiPuckAction {
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum DroneAction {
    #[serde(rename = "Power on UpCore")]
    UpCorePowerOn,
    #[serde(rename = "Shutdown UpCore")]
    UpCoreShutdown,
    #[serde(rename = "Power off UpCore")]
    UpCorePowerOff,
    #[serde(rename = "Power on Pixhawk")]
    PixhawkPowerOn,
    #[serde(rename = "Power off Pixhawk")]
    PixhawkPowerOff,
}

// serde lower case
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum GuiMessage {
    Update(String),
    // uuid, action
    Drone(uuid::Uuid, DroneAction),
    // uuid, action
    PiPuck(uuid::Uuid, PiPuckAction),
    //
    Emergency,
}

#[derive(Debug)]
struct UpCore {}

// If the xbee object doesn't exist, the drone shouldn't exist
// Doesn't cover the Pixhawk state (powered/unpowered)
#[derive(Debug)]
enum DroneState {
    Standby,
    Ready(UpCore)
}

#[derive(Debug)]
struct Drone {
    uuid: uuid::Uuid,
    xbee: Xbee,
    state: DroneState,
}

impl Drone {
    /* for consideration */
    /* don't make new async, create xbee just with ip address and set the xbee and upcore to None */
    /* add a seperate function for connecting and syncing the state of the drone */
    fn new(xbee: Xbee) -> Self {
        Drone {
            uuid: uuid::Uuid::new_v4(),
            xbee: xbee,
            state: DroneState::Standby,
        }
    }

    /* use xbee to power on device, configure, start SSH session */
    async fn connect(&self) -> Result<(), std::io::Error> {

        // transition to Ready state that includes the SSH connection
        Ok(())
    }

    /* base on the current state, create a vec of valid actions */
    fn actions(&self) -> Vec<DroneAction> {
        match self.state {
            DroneState::Standby => {
                vec![DroneAction::UpCorePowerOn]
            }
            DroneState::Ready(_) => {
                vec![DroneAction::UpCorePowerOff, DroneAction::UpCoreShutdown]
            }
        }
    }
}

const UPCORE_POWER_BIT_INDEX: u8 = 11;
const PIXHAWK_POWER_BIT_INDEX: u8 = 12;
const MUX_CONTROL_BIT_INDEX: u8 = 4;

#[derive(Debug)]
struct Xbee {
    ip: Ipv4Addr,
    last_seen: SystemTime,
    socket: Option<tokio::net::UdpSocket>,
    upcore_power: Option<bool>,
    pixhawk_power: Option<bool>,
}

impl Xbee {
    fn new(xbee_ip: Ipv4Addr) -> Self {
        Xbee {
            ip: xbee_ip,
            last_seen: SystemTime::now(),
            socket: None,
            upcore_power: None,
            pixhawk_power: None,
        }
    }

    async fn connect(&mut self) -> std::io::Result<()> {
        if let None = self.socket {
            let socket = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
            socket.connect((self.ip, 3054)).await?;
            self.socket = Some(socket);
        }
        Ok(())
    }

    async fn init(&mut self) -> std::io::Result<()> {
        if let Some(socket) = &mut self.socket {
            /* configure pins: 0 => disabled, 4 => digital output */
            let pin_disable_output: u8 = 0;
            let pin_digital_output: u8 = 4;
            /* disabled pins */
            /* these pins are the UART pins which need to be disabled for the moment */
            /* D7 -> CTS, D6 -> RTS, P3 -> DOUT, P4 -> DIN */
            socket.send(&at_command("D7", &pin_disable_output.to_be_bytes())).await?;
            socket.send(&at_command("D6", &pin_disable_output.to_be_bytes())).await?;
            socket.send(&at_command("P3", &pin_disable_output.to_be_bytes())).await?;
            socket.send(&at_command("P4", &pin_disable_output.to_be_bytes())).await?;
            /* digital output pins */
            socket.send(&at_command("D4", &pin_digital_output.to_be_bytes())).await?;
            socket.send(&at_command("P1", &pin_digital_output.to_be_bytes())).await?;
            socket.send(&at_command("P2", &pin_digital_output.to_be_bytes())).await?;
            /* set the mux to control the pixhawk from the upcore */
            /* this needs to be done before powering on the upcore */
            let mut dio_config: u16 = 0b0000_0000_0000_0000;
            let mut dio_set: u16 = 0b0000_0000_0000_0000;
            dio_config |= 1 << MUX_CONTROL_BIT_INDEX;
            dio_set |= 1 << MUX_CONTROL_BIT_INDEX;
            socket.send(&at_command("OM", &dio_config.to_be_bytes())).await?;
            socket.send(&at_command("IO", &dio_set.to_be_bytes())).await?;
            Ok(())
        }
        else {
            Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Xbee not connected"))
        }
    }

    async fn set_power(&mut self, upcore: Option<bool>, pixhawk: Option<bool>) -> std::io::Result<()> {
        if let Some(socket) = &mut self.socket {
            let mut dio_config: u16 = 0b0000_0000_0000_0000;
            let mut dio_set: u16 = 0b0000_0000_0000_0000;
            /* enable upcore power? */
            if let Some(enable_upcore_power) = upcore {
                dio_config |= 1 << UPCORE_POWER_BIT_INDEX;
                if enable_upcore_power {
                    dio_set |= 1 << UPCORE_POWER_BIT_INDEX;
                }
            }
            /* enable upcore power? */
            if let Some(enable_pixhawk_power) = pixhawk {
                dio_config |= 1 << PIXHAWK_POWER_BIT_INDEX;
                if enable_pixhawk_power {
                    dio_set |= 1 << PIXHAWK_POWER_BIT_INDEX;
                }
            }
            socket.send(&at_command("OM", &dio_config.to_be_bytes())).await?;
            socket.send(&at_command("IO", &dio_set.to_be_bytes())).await?;
            Ok(())
        }
        else {
            Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "Xbee not connected"))
        } 
       
    }
}

type Drones = Arc<RwLock<Vec<Drone>>>;

#[derive(Serialize, Debug)]
struct GuiCard {
    span: u8,
    title: String,
    content: String,
    actions: Vec<DroneAction>
}

#[derive(Serialize, Debug)]
struct GuiContent {
    title: String,
    cards: HashMap<String, GuiCard>
}

#[tokio::main]
async fn main() {
    let drones = Drones::default();
    let drones_clone = drones.clone();

    // why does this clone of the hash map need to be wrapped up as warp filter?
    let drones_filter = warp::any().map(move || drones_clone.clone());

    let index_route = warp::path::end().and(warp::fs::file(
        "/home/mallwright/Workspace/mns-supervisor/index.html",
    ));
    let static_route =
        warp::path("static").and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(drones_filter)
        .map(|ws: warp::ws::Ws, drones : Drones| {
            // This will call our function if the handshake succeeds.
            ws.on_upgrade(move |socket| connected(socket, drones))
        });

    // does the 'async move' all referenced variables into the lambda?
    let task_server = tokio::task::spawn(async move { 
        warp::serve(index_route.or(static_route).or(socket_route))
        .run(([127, 0, 0, 1], 3030))
        .await
    });

    let task_discover = tokio::task::spawn(async move {
        xbee_discover(drones).await
    });

    let (server_task_res, discover_task_res) = tokio::join!(task_server, task_discover);

    if let Err(err) = server_task_res {
        eprintln!("Joining server task failed: {}", err);
    }

    match discover_task_res {
        Err(err) => eprintln!("Joining discover task failed: {}", err),
        Ok(discover_res) => {
            if let Err(err) = discover_res {
                eprintln!("Error from the discover task: {}", err)
            }
        }
    }
}

// should this be a "member" of the drones class?
// since we have to search through the drones, it doesn't really feel like it
// collection could be just the xbee devices, which are borrowed by the drones?
// this could then be a static method over all xbee devices
async fn xbee_discover(drones: Drones) -> Result<(), std::io::Error> {
    let bind_addr =
        std::net::SocketAddr::from(([0, 0, 0, 0], 0));
    let bcast_addr =
        std::net::SocketAddr::from(([192, 168, 1, 255], 3054));
    /* create a new socket by binding */
    let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
    println!("Listening on: {}", socket.local_addr()?);
    socket.set_broadcast(true)?;

    let (mut socket_rx, mut socket_tx) = socket.split();

    /* start a tasklet to periodically send broadcasts to Xbee modules */
    let drones_clone = drones.clone();

    let handle = tokio::task::spawn(async move { 
        let drones = drones_clone;
        let packet = at_command("MY", &[]);
        loop {
            delay_for(Duration::from_millis(100)).await;
            if let Err(err) = socket_tx.send_to(&packet, &bcast_addr).await {
                /* a signal should be set here to shut everything else down */
                return err;
            }
            /* remove xbee devices which have not responded for more than half a second */
            drones.write().await.retain(|drone| {
                if let Ok(duration) = SystemTime::now().duration_since(drone.xbee.last_seen) {
                    duration.as_secs_f32() < 0.5
                }
                else {
                    true
                }
            });
        }
    });
    
    let mut rx_buffer = BytesMut::new();
    rx_buffer.resize(32, 0);
    while let Ok((bytes, client)) = socket_rx.recv_from(&mut rx_buffer).await {
        if let std::net::SocketAddr::V4(socket) = client {
            let mut drones = drones.write().await;
            if let Some(drone) = drones
                .iter_mut()
                .find(|drone| {
                    socket.ip() == &drone.xbee.ip
                }) {
                /* update the xbee's last seen timestamp */
                drone.xbee.last_seen = SystemTime::now();
            }
            else {
                /* note that here we are blocking Arc<RwLock<Vec<Drones>>> while connecting our socket */
                let mut xbee = Xbee::new(socket.ip().clone());
                /* TODO: DO NOT BLOCK the collection while connecting */
                xbee.connect().await?;
                xbee.init().await?;
                let drone = Drone::new(xbee);
                drones.push(drone);
            }
        }
    }
   
    match handle.await {
        /* Err is a join error */
        Err(_) => Ok(()),
        /* Ok(std::io::Error) an error that broke the loop */
        Ok(err) => Err(err)
    }
}

fn at_command(
    command: &str,
    arguments: &[u8]
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(10 + command.len() + arguments.len());
     /* preamble */
     packet.put_u16(0x4242);
     packet.put_u16(0x0000);
     /* packet id */
     packet.put_u8(0x00);
     /* encryption */
     packet.put_u8(0x00);
     /* command id (remote AT command) */
     packet.put_u8(0x02);
     /* command options (none) */
     packet.put_u8(0x00);
     /* frame id */
     packet.put_u8(0x01);
     /* config options (apply immediately) */
     packet.put_u8(0x02);
     /* at command */
     packet.put(command.as_bytes());
     /* at command arguments */
     packet.put(arguments);
     /* return the vector as the result */
     packet
}


async fn connected(ws: WebSocket, drones: Drones) {
    // Use a counter to assign a new unique ID for this user.

    eprintln!("websocket connected!");

    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("websocket send error: {}", e);
        }
    }));

    // this loop is basically our gui updating thread
    while let Some(data) = user_ws_rx.next().await {
        let message = match data {
            Ok(message) => message,
            Err(error) => {
                eprintln!("websocket receive error {}", error);
                break;
            }
        };

        if let Ok(message) = message.to_str() {
            if let Ok(action) = serde_json::from_str::<GuiMessage>(message) {
                match action {
                    GuiMessage::Update(view) => {
                        //eprintln!("selected view = {}", view);
            
                        let mut content = GuiContent {
                            title: String::from("Connections"),
                            cards: HashMap::new(),
                        };
            
                        for drone in drones.read().await.iter() {
                            let card = GuiCard {
                                span: 4,
                                title: String::from("Drone"),
                                content: format!("{:?}", drone),
                                // the actions depend on the state of the drone
                                // the action part of the message must contain
                                // the uuid, action name, and optionally arguments
                                actions: drone.actions(),
                            };
                            content.cards.insert(drone.uuid.to_string(), card);
                        }
            
                        if let Ok(reply) = serde_json::to_string(&content) {
                            //eprintln!("{} -> {}", view, reply);
                            if let Err(_) = tx.send(Ok(Message::text(reply))) {
                                // do nothing?
                            }
                        }
                    },
                    GuiMessage::Drone(uuid, action) => {
                        if let Some(drone) = 
                            drones.write().await
                                  .iter_mut()
                                  .find(|drone| drone.uuid == uuid) {
                            /* check if the action is still valid given the drones current state */
                            if drone.actions().contains(&action) {
                                let result = match action {
                                    DroneAction::UpCorePowerOn => {
                                        // TODO, is it possible to not block the collection here?
                                        // What if just the xbee was locked during the await?
                                        drone.xbee.set_power(Some(true), None).await
                                    },
                                    _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not implemented"))
                                };
                                eprintln!("{:?}", result);
                            }
                        }
                    },
                    _ => {
                        // TODO handle drone and pipuck messages
                    }
                }
            }
            else {
                eprintln!("[warning] could not deserialize message");
            }
        }
    }

    eprintln!("websocket disconnected!");
}
