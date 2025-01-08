mod authorization;
mod client;
mod scores;

pub use self::{
    client::{FetchResult, Osu},
    scores::{Score, Scores, ScoresDeserializer},
};
