use std::sync::RwLock;

use chrono::prelude::*;

use json::{object, JsonValue};

pub mod ac_coms;

#[derive(Debug)]
pub struct DbEntry(Class, RwLock<Status>, RwLock<DateTime<Utc>>);

/// Hold metadata for a class, we need to serve this over the API.
#[derive(knuffel::Decode, Debug)]
pub struct Class {
    #[knuffel(argument)]
    name: String,
    #[knuffel(argument)]
    url: String,
    #[knuffel(children(name = "meeting"))]
    meetings: Vec<SmallDate>,
}

/// A SmallDate is a recurring time for a meeting. Times are hard, so we cheat a bit.
#[derive(knuffel::Decode, Default, Debug)]
struct SmallDate {
    /// Three day weekday specifier.    
    #[knuffel(property)]
    day: String,

    /// HH:MM, 24-hour time, ALWAYS America/Los_Angeles. (Either PST or PDT.)    
    #[knuffel(property)]
    time: String,
}

/// We can scrape all of these from where the canvas link redirects, and then fill them into the
/// the web socket. What are each of these? No idea! But we need them, so here we are.
#[derive(Clone)]
pub struct RoomParams {
    ticket: String,
    origin: String,
    app_instance: String,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Status {
    Open,
    Closed,
    Blocked,
    Pending,
}

impl DbEntry {
    pub fn name(&self) -> String {
        self.0.name.clone()
    }

    pub fn url(&self) -> String {
        self.0.url.clone()
    }

    pub fn status(&self) -> Status {
        *self.1.read().unwrap()
    }

    /// Update the contained status and set teh contained time to that of the call. Note that this
    /// provides interior mutability.
    pub fn set_status(&self, new_status: Status) {
        let mut status = self.1.write().unwrap();
        *status = new_status;
    }

    pub fn json(&self) -> object::Object {
        let status = self.1.read().unwrap();
        let time = self.2.read().unwrap();

        let mut obj = object::Object::new();
        obj.insert("name", JsonValue::String(self.name()));
        obj.insert("times", self.0.times_json());
        obj.insert("status", status.json());
        obj.insert("last_changed", JsonValue::String(format!("{}", &*time)));

        obj
    }
}

impl From<Class> for DbEntry {
    fn from(class: Class) -> Self {
        Self(class, RwLock::new(Status::Pending), RwLock::new(Utc::now()))
    }
}

impl Status {
    pub fn json(&self) -> JsonValue {
        JsonValue::String(
            match self {
                &Self::Open => "open",
                &Self::Closed => "closed",
                &Self::Blocked => "blocked",
                &Self::Pending => "pending",
            }
            .to_owned(),
        )
    }
}

impl Class {
    pub fn times_json(&self) -> JsonValue {
        JsonValue::Array(
            self.meetings
                .iter()
                .map(|meeting| {
                    JsonValue::Object({
                        let mut obj = object::Object::new();
                        obj.insert("day", JsonValue::String(meeting.day.clone()));
                        obj.insert("time", JsonValue::String(meeting.time.clone()));

                        obj
                    })
                })
                .collect(),
        )
    }
}
