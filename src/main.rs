use bytes::{BytesMut, BufMut};
use std::time::{Duration, SystemTime};
use futures::{FutureExt, StreamExt};
use tokio::{sync::mpsc, sync::RwLock, net, time::delay_for}; //RwLock
use warp::ws::{Message, WebSocket};
use warp::Filter;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, collections::HashMap};

#[derive(Debug)]
struct XbeeDevice {
    id: uuid::Uuid,
    last_seen: SystemTime,
}

impl XbeeDevice {
    fn new(id: uuid::Uuid) -> Self {
        XbeeDevice {
            id: id,
            last_seen: SystemTime::UNIX_EPOCH,
        }
    }
}

type XbeeDevices = Arc<RwLock<HashMap<[u8; 4], XbeeDevice>>>;

#[derive(Serialize, Deserialize, Debug)]
struct GuiCard {
    span: u8,
    title: String,
    text: String,
    actions: Vec<String>
}

#[derive(Serialize, Deserialize, Debug)]
struct GuiContent {
    title: String,
    cards: Vec<GuiCard>
}

#[tokio::main]
async fn main() {
    let xbee_devices = XbeeDevices::default();
    let xbee_devices_clone = xbee_devices.clone();

    // why does this clone of the hash map need to be wrapped up as warp filter?
    let xbee_devices_filter = warp::any().map(move || xbee_devices_clone.clone());

    let index_route = warp::path::end().and(warp::fs::file(
        "/home/mallwright/Workspace/mns-supervisor/index.html",
    ));
    let static_route =
        warp::path("static").and(warp::fs::dir("/home/mallwright/Workspace/mns-supervisor/static"));
    let socket_route = warp::path("socket")
        .and(warp::ws())
        .and(xbee_devices_filter)
        .map(|ws: warp::ws::Ws, xbee_devices : XbeeDevices| {
            // This will call our function if the handshake succeeds.
            ws.on_upgrade(move |socket| connected(socket, xbee_devices))
        });

    // does the 'async move' all referenced variables into the lambda?
    let task_server = tokio::task::spawn(async move { 
        warp::serve(index_route.or(static_route).or(socket_route))
        .run(([127, 0, 0, 1], 3030))
        .await
    });

    let task_discover = tokio::task::spawn(async move {
        let bind_addr =
            std::net::SocketAddr::from(([0, 0, 0, 0], 0));
        let bcast_addr =
            std::net::SocketAddr::from(([192, 168, 1, 255], 3054));
        xbee_discover(bind_addr, bcast_addr, xbee_devices).await
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

async fn xbee_discover(
    bind_addr: std::net::SocketAddr,
    bcast_addr: std::net::SocketAddr,
    xbee_devices: XbeeDevices
) -> Result<(), std::io::Error> {
    /* create a new socket by binding */
    let socket = net::UdpSocket::bind(bind_addr).await?;
    println!("Listening on: {}", socket.local_addr()?);
    socket.set_broadcast(true)?;

    let (mut socket_rx, mut socket_tx) = socket.split();

    /* start a tasklet to periodically send broadcasts to Xbee modules */
    let xbee_devices_clone = xbee_devices.clone();

    let handle = tokio::task::spawn(async move { 
        let xbee_devices = xbee_devices_clone;
        loop {
            delay_for(Duration::from_millis(100)).await;
            if let Err(err) = xbee_command(&mut socket_tx, &bcast_addr, &b"MY"[..], &[]).await {
                /* a signal should be set here to shut everything else down */
                return err;
            }
            /* remove xbee devices which have not responded for more than half a second */
            xbee_devices.write().await.retain(|_, device| {
                if let Ok(duration) = SystemTime::now().duration_since(device.last_seen) {
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
        let rx_data = &rx_buffer[0 .. bytes];
        // TODO https://stackoverflow.com/questions/64116761
        if let [.., a, b, c, d] = rx_data {
            let mut xbee_devices = xbee_devices.write().await;
            xbee_devices
                .entry([*a,*b,*c,*d])
                .or_insert_with(|| {
                    XbeeDevice::new(uuid::Uuid::new_v4())
                }).
                last_seen = SystemTime::now();
        }
        //eprintln!("recieved {} bytes from {:?}: {:?}", bytes, client, rx_data);
    }
   
    match handle.await {
        /* Err is a join error */
        Err(_) => Ok(()),
        /* Ok(std::io::Error) an error that broke the loop */
        Ok(err) => Err(err)
    }
}

async fn xbee_command(
    socket: &mut net::udp::SendHalf,
    target: &std::net::SocketAddr,
    command: &[u8],
    arguments: &[u8],
) -> Result<usize, std::io::Error> {
    let mut packet = 
        BytesMut::with_capacity(10 + command.len() + arguments.len());
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
    packet.put(command);
    /* at command arguments */
    packet.put(arguments);
    /* send the data and return the result */
    socket.send_to(&packet, target).await
}

async fn connected(ws: WebSocket, xbee_devices: XbeeDevices) {
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
    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error: {}", e);
                break;
            }
        };

        if let Ok(view) = msg.to_str() {
            eprintln!("selected view = {}", view);
            
            let mut content = GuiContent {
                title: String::from("Connections"),
                cards: Vec::new(),
            };

            for (ip, device) in xbee_devices.read().await.iter() {
                let card = GuiCard {
                    span: 4,
                    title: format!("Xbee {:?}", ip),
                    text: format!("{:?}", device),
                    actions: vec![String::from("Connect")],
                };
                content.cards.push(card);
            }

            if let Ok(reply) = serde_json::to_string(&content) {
                if let Err(_) = tx.send(Ok(Message::text(reply))) {
                    // do nothing?
                }
            }
        }       
    }

    eprintln!("websocket disconnected!");
}
