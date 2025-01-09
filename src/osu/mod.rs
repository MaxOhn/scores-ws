mod authorization;
mod client;
mod scores;

pub use self::{
    client::{FetchResult, Osu},
    scores::{Deserializer as ScoresDeserializer, Score, Scores},
};
