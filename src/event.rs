use std::fmt::{Display, Formatter, Result as FmtResult};

use tokio_tungstenite::tungstenite::Message;

pub enum Event {
    Connect,
    Resume { score_id: u64 },
}

impl Event {
    fn parse_score_id(bytes: &[u8]) -> Option<u64> {
        bytes.iter().try_fold(0, |id, &byte| match byte {
            b'0'..=b'9' => Some(id * 10 + u64::from(byte & 0xF)),
            _ => None,
        })
    }
}

impl TryFrom<Message> for Event {
    type Error = EventError;

    fn try_from(msg: Message) -> Result<Self, Self::Error> {
        let bytes: &[u8] = match msg {
            Message::Text(ref bytes) => bytes.as_bytes(),
            Message::Binary(ref bytes) => bytes,
            _ => return Err(EventError::Variant),
        };

        if bytes == b"connect" {
            Ok(Self::Connect)
        } else if let Some(score_id) = Self::parse_score_id(bytes) {
            Ok(Self::Resume { score_id })
        } else {
            Err(EventError::Bytes)
        }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub enum EventError {
    Bytes,
    Variant,
}

impl Display for EventError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            EventError::Bytes => {
                f.write_str("message must be either `\"connect\"` \r a score id to resume from")
            }
            EventError::Variant => f.write_str("message must contain text data"),
        }
    }
}
