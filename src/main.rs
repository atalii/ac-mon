use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use anyhow::Result;
use env_logger;
use knuffel;
use log::{error, info, warn};

use json::JsonValue;

use warp;
use warp::Filter;

use tokio::time;
use tokio::time::Duration;

use ac_mon::ac_coms::AcSocket;
use ac_mon::{Class, DbEntry, RoomParams};

type Database = Arc<HashMap<String, Arc<DbEntry>>>;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let db = read_db()?;

    tokio::join!(
        serve(db.clone()),
        async {
            monitor(db).await.expect("AC monitoring failed.");
        },
    );

    Ok(())
}

async fn serve(db: Database) {
    info!("Web server task started.");

    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST"])
        .allow_headers(vec!["Authorization"]);

    let all = warp::path!("api" / "v1" / "all").map({
        let db = db.clone();
        move || all(db.clone())
    }).with(cors.clone());

    let read = warp::path!("api" / "v1" / "room" / String).map({
        let db = db.clone();
        move |name| read(db.clone(), name)
    }).with(cors.clone());

    let monitor = warp::path!("api" / "v1" / "status")
        .map(|| "".to_string())
        .with(cors.clone());

    let routes = all.or(read).or(monitor);
    warp::serve(routes).run(([0, 0, 0, 0], 8080)).await;
}

fn read(db: Database, name: String) -> String {
    let val = db.get(&name);
    match val {
        Some(val) => format!(r#"{{"error":"none","room":{}}}"#, val.json().dump()),
        None => r#"{"error": "room not found"}"#.to_owned(),
    }
}

fn all(db: Database) -> String {
    format!(
        r#"{{"rooms":{}}}"#,
        JsonValue::Array(db.values().map(|x| JsonValue::Object(x.json())).collect()).dump(),
    )
}

fn read_db() -> Result<Database> {
    let conf = fs::read_to_string("test-conf.kdl")?;

    let entries: Vec<DbEntry> = knuffel::parse::<Vec<Class>>("", &conf)?
        .into_iter()
        .map(|x| x.into())
        .collect();

    let mut db = HashMap::new();
    for entry in entries {
        db.insert(entry.name(), Arc::new(entry));
    }

    Ok(Arc::new(db))
}

async fn monitor(db: Database) -> Result<()> {
    info!("Monitor task started.");

    let mut tasks = Vec::new();

    for (_, entry) in &*db {
        let entry = entry.clone();

        info!("monitoring: {}", entry.name());

        let url = entry.url();
        let mut room_params = RoomParams::from_canvas_slug(&url).await.unwrap();

        tasks.push(tokio::spawn(async move {
            loop {
                let mut web_socket = match AcSocket::new(room_params.clone(), entry.clone()).await {
                    Ok(sock) => sock,
                    Err(e) => {
                        error!("failed to create socket for {}: {}", entry.name(), e);
                        break;
                    }
                };

                if web_socket.listen().await {
                    info!("sleeping: {}", entry.name());
                    web_socket.close().await;
                    time::sleep(Duration::from_secs(15 * 60)).await;
                } else {
                    info!("failed, restarting: {}", entry.name());
                }

                loop {
                    match RoomParams::from_canvas_slug(&url).await {
                        Ok(rp) => {
                            room_params = rp;
                            break;
                        }

                        Err(e) => {
                            warn!("Couldn't get room params: {}", e);
                            time::sleep(Duration::from_secs(10)).await;
                        }
                    };
                }
            }
        }));
    }

    for task in tasks {
        task.await?;
    }

    Ok(())
}
