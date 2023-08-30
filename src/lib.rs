use std::{thread, time};

use serde::{Deserialize, Serialize};

pub const BUFSIZE: usize = 8 * 1024; // 8 KiB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub username: String,
    pub content: Vec<u8>,
}

impl Message {
    pub fn serialize(&self) -> bincode::Result<Vec<u8>> {
        bincode::serialize(self)
    }
}

impl<'a> TryFrom<&'a [u8]> for Message {
    type Error = Box<bincode::ErrorKind>;

    fn try_from(value: &'a [u8]) -> bincode::Result<Self> {
        bincode::deserialize(value)
    }
}

pub fn sleep(millis: u64) {
    let duration = time::Duration::from_millis(millis);
    thread::sleep(duration);
}
