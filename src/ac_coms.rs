//! This module contains all the logic necessary to acquire a connection to and communicate over a
//! WebSocket with the Adobe Connect servers.

use anyhow::Result;

use futures_util::sink::SinkExt;
use futures_util::StreamExt;

use json;

use log::{debug, info, warn};

use regex::Regex;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use tokio::net::TcpStream;

use tokio_tungstenite;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::{DbEntry, RoomParams, Status};

const RTMP_SLUG: &'static str = "rtmps://spcs-app3uswest1.acms.com:443/";
const SWF_SLUG: &'static str = "https://pcadobeconnect.stanford.edu/common/webrtchtml/index.html";
const WS_LOC: &'static str = "wss://amsprod-connect-uswest1-acts1.acms.com:443/";

#[derive(Error, Debug)]
pub enum InitError {
    #[error("Ticket extraction failed: {0}.")]
    TicketExtractionFail(String),

    #[error("Origin extraction failed.")]
    OriginExtractionFail,

    #[error("App instance extraction failed.")]
    AppExtractionFail,

    #[error("Adobe rejected the connection to the web socket.")]
    UnsuccessfulWs,
}

#[derive(Error, Debug)]
pub enum CommError {
    #[error("The received RPC was not valid JSON.")]
    InvalidJson,

    #[error("The received RPC requseted an unknown command.")]
    UnknownCommand,

    #[error("The received RPC requested an unknown method.")]
    UnknownMethod,

    #[error("Name attribute missing from RPC.")]
    MissingName,

    #[error("Missing a parameter from RPC.")]
    MissingParams,

    #[error("Received an empty message.")]
    Empty,
}

/// Encapsulate an AC socket connection.
pub struct AcSocket {
    inner: WebSocketStream<MaybeTlsStream<TcpStream>>,
    entry: Arc<DbEntry>,
}

/// AC's websocket sends a series of JSON RPC requests to us. When and what commands are sent
/// indirectly encodes the data we're looking for.
#[derive(Debug)]
struct Rpc(String);

impl AcSocket {
    /// Create and initialize a connection with the Adobe Connect web socket, connecting to the
    /// specified room.
    pub async fn new(room_params: RoomParams, entry: Arc<DbEntry>) -> Result<Self> {
        let (mut inner, _) = tokio_tungstenite::connect_async(WS_LOC).await?;

        let msg = room_params.init_rpc_msg();
        inner
            .send(tokio_tungstenite::tungstenite::Message::Text(msg))
            .await?;

        let status = inner.next().await.unwrap().unwrap().into_text().unwrap();
        let status = json::parse(&status)?;
        let status = match status {
            json::JsonValue::Object(o) => o,
            _ => panic!("ahahahaha fuck"),
        };

        let status = status
            .get("status")
            .and_then(|x| match x {
                json::JsonValue::Object(o) => o.get("code"),
                _ => None,
            })
            .and_then(|x| x.as_str());

        if status != Some("NetConnection.Connect.Success") {
            return Err((InitError::UnsuccessfulWs).into());
        }

        inner
            .send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"type":"WSFunc","method":"startHeartbeat","value":true}"#.to_string(),
            ))
            .await?;

        Ok(Self { inner, entry })
    }

    /// Listen on a websocket. Return true if the room opens, and false if the socket gives out
    /// before that.
    pub async fn listen(&mut self) -> bool {
        let mut status = Status::Pending;

        while status == Status::Closed || status == Status::Pending {
            let response = self.inner.next().await;

            let next = match response {
                Some(k) => k,
                None => return false,
            };

            let next = match next {
                Ok(k) => k.into_text().unwrap(),
                Err(_) => return false,
            };

            match Rpc::new(&next) {
                Err(e) => warn!(
                    "Unable to handle RPC from: {}; ignoring: {}",
                    self.entry.name(),
                    e,
                ),

                Ok(rpc) if rpc.is_heartbeat() => {
                    debug!("received heartbeat from: {}", self.entry.name())
                }

                Ok(rpc) if rpc.is_timeout() => {
                    info!("timed out: {}", self.entry.name());
                    return false;
                }

                Ok(rpc) => {
                    status = rpc.update(status);
                    self.entry.set_status(status);
                    info!("room changed: {}/{:?}", self.entry.name(), status);
                }
            };
        }

        return true;
    }

    pub async fn close(&mut self) {
        if let Err(e) = self.inner.close(None).await {
            warn!("Couldn't close socket: {e}");
        }
    }
}

impl RoomParams {
    /// Scrape required information from the given canvas stub URL. Our scraping isn't actually    
    /// proper parsing, we just use a regex. Yeah... it's... not great. (Do note that we're looking    
    /// through javascript, so we can't actually just replace this with a proper HTML parser.)    
    pub async fn from_canvas_slug(url: &'_ str) -> Result<Self> {
        let ticket_pattern = Regex::new(r"ticket%3D(?P<ticket>[a-z0-9]+)%26").unwrap();
        let origin_pattern =
            Regex::new(r"origins%3D(?P<origins>[a-z0-9\-]+)%3A(?P<extra>[0-9]+)%2C").unwrap();

        let app_pattern = Regex::new(r"appInstance%3D(?P<app>[0-9]%2F[0-9A-F]+)%2F").unwrap();

        let body = reqwest::get(url).await?.text().await?;

        let ticket = ticket_pattern
            .captures_iter(&body)
            .next()
            .map(|x| x["ticket"].to_owned())
            .ok_or(InitError::TicketExtractionFail(url.to_owned()))?;

        let origin = origin_pattern
            .captures_iter(&body)
            .next()
            .map(|x| format!("{}:{}", &x["origins"], &x["extra"]))
            .ok_or(InitError::OriginExtractionFail)?;

        let app_instance = app_pattern
            .captures_iter(&body)
            .next()
            .map(|x| x["app"].to_owned())
            .ok_or(InitError::AppExtractionFail)?;

        Ok(Self {
            ticket,
            origin,
            app_instance,
        })
    }

    /// Get the JSON request to send over the AC websocket.    
    pub fn init_rpc_msg(&self) -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Couldn't get proper time.")
            .as_millis();

        let ticket = &self.ticket;
        let origin = &self.origin;
        let app_instance = &self.app_instance;

        let mut json = format!(
            r#"
{{
    "type": "NCFunc",
    "method": "connect",    
    "url": "{RTMP_SLUG}?rtmp://{origin}/meetingas3app/{app_instance}/",    
    "params": {{
        "ticket": "{ticket}",
        "reconnection": false,
        "swfUrl": "{SWF_SLUG}?timestamp={timestamp}",        
        "Recording": false
    }}
}}"#
        );

        json.retain(|c| c != '\n' && c != ' ');
        json
    }
}

impl Rpc {
    /// Extract and validate anything useful from some JSON. (Might be good to parse-not-validate    
    /// later, but this isn't too bad for now.)    
    pub fn new(json: &str) -> Result<Self> {
        debug!("RCV'd: {}", json);

        if json.is_empty() {
            Err(CommError::Empty)?;
        }

        let json = json::parse(json).map_err(|_| CommError::InvalidJson)?;
        let rpc = match json {
            json::JsonValue::Object(o) => Ok(o),
            _ => Err(CommError::InvalidJson),
        }?;

        let method = rpc.get("method").ok_or(CommError::UnknownMethod)?;

        if method == "heartbeat" {
            return Ok(Self("heartbeat".to_owned()));
        }

        if method != "onCommand" {
            Err(CommError::UnknownMethod)?;
        }

        let command = rpc.get("command").ok_or(CommError::UnknownCommand)?;

        if command != "loginHandler" {
            Err(CommError::UnknownCommand)?;
        }

        let params = rpc.get("params").ok_or(CommError::MissingParams)?;
        let params = match params {
            json::JsonValue::Object(o) => Ok(o),
            _ => Err(CommError::InvalidJson),
        }?;

        let arg = params.get("arg_0").ok_or(CommError::MissingParams)?;
        let arg = match arg {
            json::JsonValue::Object(o) => Ok(o),
            _ => Err(CommError::InvalidJson),
        }?;

        let command = arg
            .get("command")
            .ok_or(CommError::MissingName)?
            .as_str()
            .unwrap()
            .to_owned();

        Ok(Self(command))
    }

    pub fn is_heartbeat(&self) -> bool {
        self.0 == "heartbeat"
    }

    pub fn is_timeout(&self) -> bool {
        self.0 == "connectionTimedOut"
    }

    pub fn update(&self, status: Status) -> Status {
        info!("Updating: {}", self.0);
        match status {
            Status::Closed | Status::Pending => match &self.0[..] {
                "accepted" => Status::Open,
                "blocked" => Status::Blocked,
                _ => Status::Closed,
            },

            Status::Open => Status::Open,
            Status::Blocked => Status::Blocked,
        }
    }
}
