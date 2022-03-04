use anyhow::Result;
use bytes::Bytes;
use clap::{App, AppSettings, Arg};
use std::fmt::Debug;
use std::io::Write;
use std::sync::Arc;
use tokio::time::Duration;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::math_rand_alpha;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use reqwest::Client as HttpClient;
use tinyjson::JsonValue;
use webrtc::dtls_transport::dtls_role::DTLSRole;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;

const MESSAGE_SIZE: usize = 1500;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter(None, log::LevelFilter::Trace)
        .init();

    // Everything below is the WebRTC-rs API! Thanks for using it ❤️.

    // Create a MediaEngine object to configure the supported codec
    let mut media_engine = MediaEngine::default();

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
    // this is enabled by default. If you are manually managing You MUST create a InterceptorRegistry
    // for each PeerConnection.
    let mut registry = Registry::new();

    // Since this behavior diverges from the WebRTC API it has to be
    // enabled using a settings engine. Mixing both detached and the
    // OnMessage DataChannel API is not supported.

    // Create a SettingEngine and enable Detach
    let mut setting_engine = SettingEngine::default();
    setting_engine.detach_data_channels();
    setting_engine.set_answering_dtls_role(DTLSRole::Client);

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    // Create a datachannel with label 'data'
    let mut data_channel_config = RTCDataChannelInit::default();
    data_channel_config.ordered = Some(false);
    data_channel_config.max_retransmits = Some(0);

    let data_channel = peer_connection
        .create_data_channel("data", Some(data_channel_config))
        .await?;

    data_channel
        .on_error(Box::new(move |error| {
            println!("data channel error!");
            Box::pin(async {
                println!("data channel error!");
            })
        }))
        .await;

    // Register channel opening handling
    let data_channel_ref = Arc::clone(&data_channel);
    data_channel
        .on_open(Box::new(move || {
            println!(
                "Data channel '{}'-'{}' open.",
                data_channel_ref.label(),
                data_channel_ref.id()
            );

            let data_channel_ref_2 = Arc::clone(&data_channel_ref);
            Box::pin(async move {
                let detached_data_channel = data_channel_ref_2
                    .detach()
                    .await
                    .expect("data channel detach got error");

                // Handle reading from the data channel
                let detached_data_channel_1 = Arc::clone(&detached_data_channel);
                let detached_data_channel_2 = Arc::clone(&detached_data_channel);
                tokio::spawn(async move {
                    read_loop(detached_data_channel_1).await;
                });

                // Handle writing to the data channel
                tokio::spawn(async move {
                    write_loop(detached_data_channel_2).await;
                });
            })
        }))
        .await;

    peer_connection
        .on_ice_candidate(Box::new(move |candidate_opt| {
            if let Some(candidate) = &candidate_opt {
                println!("received ice candidate from: {}", candidate.address);
            } else {
                println!("all local candidates received");
            }

            Box::pin(async {})
        }))
        .await;

    // Create an offer to send to the browser
    let offer = peer_connection.create_offer(None).await?;

    // Sets the LocalDescription, and starts our UDP listeners
    peer_connection.set_local_description(offer).await?;

    // Send a request to server to initiate connection
    let http_client = HttpClient::new();

    let server_url = "http://127.0.0.1:14191/rtc_session";

    let sdp = peer_connection.local_description().await.unwrap().sdp;

    let request = http_client
        .post(server_url)
        .header("Content-Length", sdp.len())
        .body(sdp);

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(err) => {
            panic!("Could not send request, original error: {:?}", err);
        }
    };
    let mut response_string = response.text().await.unwrap();

    // parse session response
    let session_response: JsSessionResponse = get_session_response(response_string.as_str());

    // apply the answer as the remote description
    let mut session_description = RTCSessionDescription::default();
    session_description.sdp_type = RTCSdpType::Answer;
    session_description.sdp = session_response.answer.sdp;
    peer_connection
        .set_remote_description(session_description)
        .await?;

    // create ice candidate
    let ice_candidate = RTCIceCandidateInit {
        candidate: session_response.candidate.candidate,
        sdp_mid: session_response.candidate.sdp_mid,
        sdp_mline_index: session_response.candidate.sdp_m_line_index,
        ..Default::default()
    };
    if let Err(error) = peer_connection.add_ice_candidate(ice_candidate).await {
        panic!("Error during add_ice_candidate: {:?}", error);
    }

    loop {}

    Ok(())

    // // Block until ICE Gathering is complete, disabling trickle ICE
    // // we do this because we only can exchange one signaling message
    // // in a production application you should exchange ICE Candidates via OnICECandidate
    // let _ = gather_complete.recv().await;
    //
    // // Output the offer in base64 so we can paste it in browser
    // if let Some(local_desc) = peer_connection.local_description().await {
    //     let json_str = serde_json::to_string(&local_desc)?;
    //     let b64 = signal::encode(&json_str);
    //     println!("{}", b64);
    // } else {
    //     println!("generate local_description failed!");
    // }
    //
    // // Wait for the answer to be pasted
    // let line = signal::must_read_stdin()?;
    // let desc_data = signal::decode(line.as_str())?;
    // let answer = serde_json::from_str::<RTCSessionDescription>(&desc_data)?;
}

// read_loop shows how to read from the datachannel directly
async fn read_loop(data_channel: Arc<webrtc::data::data_channel::DataChannel>) -> Result<()> {
    let mut buffer = vec![0u8; MESSAGE_SIZE];
    loop {
        let message_length = match data_channel.read(&mut buffer).await {
            Ok(length) => length,
            Err(err) => {
                println!("Datachannel closed; Exit the read_loop: {}", err);
                return Ok(());
            }
        };

        println!(
            "Message from DataChannel: {}",
            String::from_utf8(buffer[..message_length].to_vec())?
        );
    }
}

// write_loop shows how to write to the datachannel directly
async fn write_loop(data_channel: Arc<webrtc::data::data_channel::DataChannel>) -> Result<()> {
    let mut result = Result::<usize>::Ok(0);
    while result.is_ok() {
        let timeout = tokio::time::sleep(Duration::from_secs(5));
        tokio::pin!(timeout);

        tokio::select! {
            _ = timeout.as_mut() =>{
                let message = "PING".to_string();
                println!("Sending '{}'", message);
                result = data_channel.write(&Bytes::from(message)).await.map_err(Into::into);
            }
        };
    }

    Ok(())
}

#[derive(Clone)]
pub struct SessionAnswer {
    pub sdp: String,
    pub type_str: String,
}

pub struct SessionCandidate {
    pub candidate: String,
    pub sdp_m_line_index: u16,
    pub sdp_mid: String,
}

pub struct JsSessionResponse {
    pub answer: SessionAnswer,
    pub candidate: SessionCandidate,
}

fn get_session_response(input: &str) -> JsSessionResponse {
    let json_obj: JsonValue = input.parse().unwrap();

    let sdp_opt: Option<&String> = json_obj["answer"]["sdp"].get();
    let sdp: String = sdp_opt.unwrap().clone();

    let type_str_opt: Option<&String> = json_obj["answer"]["type"].get();
    let type_str: String = type_str_opt.unwrap().clone();

    let candidate_opt: Option<&String> = json_obj["candidate"]["candidate"].get();
    let candidate: String = candidate_opt.unwrap().clone();

    let sdp_m_line_index_opt: Option<&f64> = json_obj["candidate"]["sdpMLineIndex"].get();
    let sdp_m_line_index: u16 = *(sdp_m_line_index_opt.unwrap()) as u16;

    let sdp_mid_opt: Option<&String> = json_obj["candidate"]["sdpMid"].get();
    let sdp_mid: String = sdp_mid_opt.unwrap().clone();

    JsSessionResponse {
        answer: SessionAnswer { sdp, type_str },
        candidate: SessionCandidate {
            candidate,
            sdp_m_line_index,
            sdp_mid,
        },
    }
}
