use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use reqwest::Client as HttpClient;
use tinyjson::JsonValue;
use tokio::time::Duration;

use webrtc::{
    api::{setting_engine::SettingEngine, APIBuilder},
    data_channel::data_channel_init::RTCDataChannelInit,
    dtls_transport::dtls_role::DTLSRole,
    ice_transport::ice_candidate::RTCIceCandidateInit,
    peer_connection::{
        configuration::RTCConfiguration, sdp::sdp_type::RTCSdpType,
        sdp::session_description::RTCSessionDescription,
    },
};

const MESSAGE_SIZE: usize = 1500;

mod addr_cell;
use addr_cell::{AddrCell, ServerAddr};

#[tokio::main]
async fn main() -> Result<()> {
    // setup logging
    env_logger::Builder::new()
        .filter(None, log::LevelFilter::Info)
        .init();

    log::info!("Client Demo started");

    let addr_cell = AddrCell::default();

    // create a SettingEngine and enable Detach
    let mut setting_engine = SettingEngine::default();
    setting_engine.detach_data_channels();
    setting_engine
        .set_answering_dtls_role(DTLSRole::Client)
        .expect("error in set_answering_dtls_role!");

    // create the API object
    let api = APIBuilder::new()
        .with_setting_engine(setting_engine)
        .build();

    // create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    // create a config for our new datachannel
    let mut data_channel_config = RTCDataChannelInit::default();
    data_channel_config.ordered = Some(false);
    data_channel_config.max_retransmits = Some(0);
    data_channel_config.id = Some(0);

    // create a datachannel with label 'data'
    let data_channel = peer_connection
        .create_data_channel("data", Some(data_channel_config))
        .await?;

    // datachannel on_error callback
    data_channel
        .on_error(Box::new(move |error| {
            println!("data channel error: {:?}", error);
            Box::pin(async {
                println!("data channel error!");
            })
        }))
        .await;

    // datachannel on_open callback
    let data_channel_ref = Arc::clone(&data_channel);
    let addr_cell_ref = addr_cell.clone();
    data_channel
        .on_open(Box::new(move || {
            let data_channel_ref_2 = Arc::clone(&data_channel_ref);
            let addr_cell_ref_2 = addr_cell_ref.clone();
            Box::pin(async move {
                let detached_data_channel = data_channel_ref_2
                    .detach()
                    .await
                    .expect("data channel detach got error");

                // Handle reading from the data channel
                let detached_data_channel_1 = Arc::clone(&detached_data_channel);
                let detached_data_channel_2 = Arc::clone(&detached_data_channel);
                let detached_addr_cell_1 = addr_cell_ref_2.clone();
                let detached_addr_cell_2 = addr_cell_ref_2.clone();
                tokio::spawn(async move {
                    read_loop(detached_addr_cell_1, detached_data_channel_1)
                        .await
                        .expect("error in read_loop!");
                });

                // Handle writing to the data channel
                tokio::spawn(async move {
                    write_loop(detached_addr_cell_2, detached_data_channel_2)
                        .await
                        .expect("error in write_loop!");
                });
            })
        }))
        .await;

    // create an offer to send to the server
    let offer = peer_connection.create_offer(None).await?;

    // sets the LocalDescription, and starts our UDP listeners
    peer_connection.set_local_description(offer).await?;

    // send a request to server to initiate connection (signaling, essentially)
    let http_client = HttpClient::new();

    let server_url = "http://127.0.0.1:14191/rtc_session";

    let sdp = peer_connection.local_description().await.unwrap().sdp;

    let request = http_client
        .post(server_url)
        .header("Content-Length", sdp.len())
        .body(sdp);

    // wait to receive a response from server
    let response = match request.send().await {
        Ok(resp) => resp,
        Err(err) => {
            panic!("Could not send request, original error: {:?}", err);
        }
    };
    let response_string = response.text().await.unwrap();

    // parse session from server response
    let session_response: JsSessionResponse = get_session_response(response_string.as_str());

    // apply the server's response as the remote description
    let mut session_description = RTCSessionDescription::default();
    session_description.sdp_type = RTCSdpType::Answer;
    session_description.sdp = session_response.answer.sdp;
    peer_connection
        .set_remote_description(session_description)
        .await?;

    addr_cell
        .receive_candidate(session_response.candidate.candidate.as_str())
        .await;

    // create ice candidate
    let ice_candidate = RTCIceCandidateInit {
        candidate: session_response.candidate.candidate,
        sdp_mid: Some(session_response.candidate.sdp_mid),
        sdp_mline_index: Some(session_response.candidate.sdp_m_line_index),
        ..Default::default()
    };
    // add ice candidate to connection
    if let Err(error) = peer_connection.add_ice_candidate(ice_candidate).await {
        panic!("Error during add_ice_candidate: {:?}", error);
    }

    // don't block .. I'm sure there's a better way to do this
    loop {}
}

// read_loop shows how to read from the datachannel directly
async fn read_loop(
    addr_cell: AddrCell,
    data_channel: Arc<webrtc::data::data_channel::DataChannel>,
) -> Result<()> {
    let mut buffer = vec![0u8; MESSAGE_SIZE];
    loop {
        let message_length = match data_channel.read(&mut buffer).await {
            Ok(length) => length,
            Err(err) => {
                println!("Datachannel closed; Exit the read_loop: {}", err);
                return Ok(());
            }
        };

        let addr = match addr_cell.get().await {
            ServerAddr::Found(addr) => addr.to_string(),
            ServerAddr::Finding => "".to_string(),
        };
        log::info!(
            "Client recv <- {}: {}",
            addr,
            String::from_utf8(buffer[..message_length].to_vec())?
        );
    }
}

// write_loop shows how to write to the datachannel directly
async fn write_loop(
    addr_cell: AddrCell,
    data_channel: Arc<webrtc::data::data_channel::DataChannel>,
) -> Result<()> {
    let mut result = Result::<usize>::Ok(0);
    while result.is_ok() {
        let timeout = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(timeout);

        tokio::select! {
            _ = timeout.as_mut() =>{
                let addr = match addr_cell.get().await {
                    ServerAddr::Found(addr) => addr.to_string(),
                    ServerAddr::Finding => "".to_string(),
                };
                let message = "PING".to_string();
                log::info!("Client send -> {}: {}", addr, message);
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
