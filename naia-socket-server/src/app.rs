use naia_server_socket::{Packet, PacketReceiver, PacketSender, ServerAddrs, Socket};
use naia_socket_shared::SocketConfig;

pub struct App {
    packet_sender: PacketSender,
    packet_receiver: PacketReceiver,
}

impl App {
    pub fn new() -> Self {
        info!("Naia Server Socket Demo started");

        let server_address = ServerAddrs::new(
            "127.0.0.1:14191"
                .parse()
                .expect("could not parse Session address/port"),
            // IP Address to listen on for UDP WebRTC data channels
            "127.0.0.1:14192"
                .parse()
                .expect("could not parse WebRTC data address/port"),
            // The public WebRTC IP address to advertise
            "http://127.0.0.1:14192",
        );

        let mut socket = Socket::new(SocketConfig::new(None, None));
        socket.listen(server_address);

        App {
            packet_sender: socket.packet_sender(),
            packet_receiver: socket.packet_receiver(),
        }
    }

    pub fn update(&mut self) {
        match self.packet_receiver.receive() {
            Ok(Some(packet)) => {
                let message_from_client = String::from_utf8_lossy(&packet.payload());
                info!("Server recv <- {}: {}", packet.address(), message_from_client);

                if message_from_client.eq("PING") {
                    let message_to_client: String = "PONG".to_string();
                    info!("Server send -> {}: {}", packet.address(), message_to_client);
                    self.packet_sender.send(Packet::new(
                        packet.address(),
                        message_to_client.as_bytes().into(),
                    ));
                }
            }
            Ok(None) => {}
            Err(error) => {
                info!("Server Error: {}", error);
            }
        }
    }
}
