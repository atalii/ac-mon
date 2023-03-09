use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use anyhow::Result;
use env_logger;
use knuffel;
use log::info;

use json::JsonValue;

use warp;
use warp::Filter;

use ac_mon::ac_coms::AcSocket;
use ac_mon::{Class, DbEntry, RoomParams};

type Database = Arc<HashMap<String, Arc<DbEntry>>>;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let db = read_db()?;

    tokio::join!(serve(db.clone()), monitor(db),);

    Ok(())
}

async fn serve(db: Database) {
    info!("Web server task started.");

    let all = warp::path!("api" / "v1" / "all").map({
        let db = db.clone();
        move || all(db.clone())
    });

    let read = warp::path!("api" / "v1" / "room" / String).map({
        let db = db.clone();
        move |name| read(db.clone(), name)
    });

    let routes = all.or(read);
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
        r#"{{
    "rooms": {}
}}
"#,
        JsonValue::Array(
            db.values().map(|x| JsonValue::Object(x.json())).collect()
        ).dump(),
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

        let url = entry.url();
        let room_params = RoomParams::from_canvas_slug(&url).await?;
        let mut web_socket = AcSocket::new(room_params, entry.clone()).await?;

        info!("monitoring: {}", entry.name());

        tasks.push(tokio::spawn(async move { web_socket.listen().await }));
    }

    for task in tasks {
        task.await?;
    }

    Ok(())
}
