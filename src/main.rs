use tokio::net::UdpSocket;
use tokio::net::ToSocketAddrs;
use tokio::time::delay_for;

use bytes::{BytesMut, BufMut};
//use core::num::Wrapping;
use core::time::Duration;

/*
struct Xbee {
    socket: UdpSocket,
}
impl Xbee {
    fn write_command(&mut self, command: &str, arguments: &[u8]) {}
    fn write_data(data: &[u8]) {}
}
*/

// to look into: using UdpCodec / framed to build higher level protocols
// https://dev.to/jtenner/creating-a-tokio-codec-1f0l
// https://github.com/tokio-rs/tokio/blob/master/examples/udp-codec.rs
// struct XbeeWifiCodec {}
// socket.next() ?

// DIO4 -> Green LED (RSSI) -> COM_MUX_CTRL
// DIO11 -> Up Core Enable
// DIO12 -> Pixhawk Enable

// Add WebUI https://getmdl.io/templates/index.html (dashboard) 
// built on top of https://github.com/seanmonstar/warp

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let bind_addr = "0.0.0.0:0";
    let target_addr = "192.168.1.153:3054";
    let mut socket = UdpSocket::bind(&bind_addr).await?;
    socket.connect(&target_addr).await?;
    println!("Listening on: {}", socket.local_addr()?);
    
    let pin_digital_output: u8 = 4; // DIO is a digital output (default output low)
    let mut pin_digital_output_packet = BytesMut::with_capacity(1);
    pin_digital_output_packet.put_u8(pin_digital_output);

    let pin_disable_output: u8 = 0; // DIO is disabled
    let mut pin_disable_output_packet = BytesMut::with_capacity(1);
    pin_disable_output_packet.put_u8(pin_disable_output);

    // bitmask: 4th, 11th, 12th bit are outputs (counting from zero)
    let dio_config: u16 = 0b0000_1000_0000_0000;
    let mut dio_config_packet = BytesMut::with_capacity(2);
    dio_config_packet.put_u16(dio_config);

    //COM_MUX_CTRL OFF
    //Up Core Enable ON
    //Pixhawk Enable OFF
    let dio_set: u16 = 0b0000_1000_0000_0000;
    let mut dio_set_packet = BytesMut::with_capacity(2);
    dio_set_packet.put_u16(dio_set);

    // disable UART pins
    // D7 -> CTS, D6 -> RTS, P3 -> DOUT, P4 -> DIN
    write_command(&mut socket, &target_addr, &b"D7"[..], &pin_disable_output_packet).await?;
    write_command(&mut socket, &target_addr, &b"D6"[..], &pin_disable_output_packet).await?;
    write_command(&mut socket, &target_addr, &b"P3"[..], &pin_disable_output_packet).await?;
    write_command(&mut socket, &target_addr, &b"P4"[..], &pin_disable_output_packet).await?;

    //write_command(&mut socket, &target_addr, &b"D4"[..], &pin_config_packet).await?;
    write_command(&mut socket, &target_addr, &b"P1"[..], &pin_digital_output_packet).await?;
    //write_command(&mut socket, &target_addr, &b"P2"[..], &pin_config_packet).await?;
    write_command(&mut socket, &target_addr, &b"OM"[..], &dio_config_packet).await?;
    write_command(&mut socket, &target_addr, &b"IO"[..], &dio_set_packet).await?;
    Ok(())
}

/*
#[tokio::main]
async fn main() {
    let result = discover("0.0.0.0:0", "192.168.1.255:3054").await;
    if let Ok(xbee_sockets) = result {
        println!("ok!");
        for xbee_socket in xbee_sockets {
            println!("{:?}", xbee_socket);
        }
    }
    else {
        println!("error {:?}", result);
    }
}
*/

async fn write_command<A: ToSocketAddrs>(
    socket: &mut UdpSocket,
    target: A,
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

// move socket to broadcast thread
// split the recieving part and the sending part between two threads?
// some support for this in Udp Codec?
async fn discover<A: ToSocketAddrs>(
    bind_addr: A,
    bcast_addr: A,
) -> Result<Vec<UdpSocket>, std::io::Error> {
    /* create a new socket by binding */
    let mut socket = UdpSocket::bind(bind_addr).await?;
    println!("Listening on: {}", socket.local_addr()?);
    socket.set_broadcast(true)?;
    //socket.connect(bcast_addr).await?;
    write_command(&mut socket, bcast_addr, &b"MY"[..], &[]).await?;
    delay_for(Duration::from_millis(500)).await;
    let mut packet = 
        BytesMut::with_capacity(32);
    eprintln!("A");
    if let Ok((bytes, client)) = socket.recv_from(&mut packet).await {
        println!("recieved {} bytes from {:?}", bytes, client);
    }
    eprintln!("B");
    Ok(vec![])
}
