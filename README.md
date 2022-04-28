# naia-socket-webrtc-rs-example
### Purpose:
It would be great to run a Rust server that can communicate with both Wasm & Native
clients via an unordered and unreliable transport layer right? That is a primary goal
of [naia-socket](https://github.com/naia-lib/naia/socket), and getting there includes
getting this example to work correctly! I need help on this, and so here's an
easy-to-clone example for those who might be able to help to jump into it a bit easier.

So far `naia-socket` has allowed for a cross-platform (wasm-included) UDPSocket-like
abstraction over WebRTC datachannels via
[webrtc-unreliable](https://github.com/kyren/webrtc-unreliable), and a plain
`std::net::UdpSocket` for native clients. [webrtc-rs](https://github.com/webrtc-rs/webrtc)
gives us the possibility of _getting rid of_ the `UdpSocket` and using a native WebRTC
client datachannel under the hood. This would allow servers using `naia-socket` to
connect to native AND wasm clients simultaneously.

This example demonstrates where we're at at attempting to connect a `webrtc-rs` datachannel
to `naia-socket`. Thank you so much for any help.

### Current Blocker
It appears that the DTLS handshake is failing on the
[ServerKeyExchange message deserialization step here](https://github.com/webrtc-rs/dtls/blob/611f052515e7090be661bcdd4d599f0d4c7bcdca/src/handshake/handshake_message_server_key_exchange.rs#L114).
The message indicates that the incoming signature will be something like ~19000 bytes,
but the incoming message _size_ is only ~300 bytes ... curious.

This results in the client printing a `client: handshake parse failed: buffer is too
small` message, and then restarting the DTLS handshake from the beginning. It then
loops this behavior indefinitely.

The failing handshake message is [being created here](https://github.com/kyren/webrtc-unreliable/blob/2ba6487d6e4d7f074f94d1b87fd54bd528b1cc36/src/client.rs#L235)
in `webrtc-unreliable` by an `openssl::SslAcceptor` [initiated here](https://github.com/kyren/webrtc-unreliable/blob/2ba6487d6e4d7f074f94d1b87fd54bd528b1cc36/src/crypto.rs#L56).
[OpenSSL](https://github.com/sfackler/rust-openssl), because of bindings to the C-libraries, has of course been more opaque and
harder to debug than the [webrtc-rs/dtls](https://github.com/webrtc-rs/dtls) crate..

### Possible Solution Ideas..
1. Is the ServerKeyExchange message being fragmented somehow, and the client is only
receiving the first part of the message which will contain the full ~19 kb signature?
   (seems to be the suggestion from this seemingly relevant [GitHub issue](https://github.com/confluentinc/confluent-kafka-go/issues/129)..
`Not enough data` seems to be the equivalent error message in their DTLS library)

2. Is there a DTLS server mismatch? The client is attempting to parse a different version
of an incoming ServerKeyExchange message? (seems to be suggested [here](https://stackoverflow.com/questions/56319622/dtls-handshake-failed-with-alert-after-serverhellodone))
3. ... your idea added here soon!

### Instructions to run Example:

#### Run the Server:
1. `naia-socket-server` depends on https://github.com/kyren/webrtc-unreliable
which depends on https://github.com/sfackler/rust-openssl for DTLS, so first
follow instructions to install `openssl` locally using notes at
https://docs.rs/openssl/latest/openssl/
2. 
````
cd naia-socket-server
cargo run
````

#### Run the Client:
1.
````
cd webrtc-rs-client
cargo run
````
