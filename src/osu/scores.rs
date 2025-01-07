use bytes::Bytes;
use eyre::{Context as _, Result};
use memchr::memmem;
use tokio_tungstenite::tungstenite::Message;

use std::{cmp::Ordering, collections::BTreeSet, ops::ControlFlow};

/// Deserializes the osu!api response.
///
/// The format is expected to be of the following form:
///
/// ```json
/// {
///     "scores": [{ ... }, ...],
///     "cursor": {"id": number},
///     "cursor_string": "..."
/// }
/// ```
///
/// `cursor_string` works the same as `cursor` but with an encoded number.
/// Since the cursors' number actually is the *newest* score id, we don't
/// really want that since we're interested in the *oldest* one. Hence, we skip
/// deserializing them entirely and only handle scores; then use the scores'
/// oldest id as cursor.
pub struct ScoresDeserializer {
    bytes: Bytes,
    idx: usize,
}

impl ScoresDeserializer {
    pub fn new(bytes: Bytes) -> Self {
        Self { bytes, idx: 0 }
    }

    pub fn deserialize(mut self, scores: &mut BTreeSet<Score>) -> Result<()> {
        const SCORES: &[u8] = br#""scores":"#;

        let start = memmem::find(&self.bytes, SCORES).ok_or(eyre!("missing scores"))?;
        self.idx = start + SCORES.len();

        self.deserialize_scores(scores)
            .context("failed to deserialize scores")
    }

    fn deserialize_scores(&mut self, scores: &mut BTreeSet<Score>) -> Result<()> {
        let start = Self::skip_whitespace_until(&self.bytes[self.idx..], |byte| byte == b'[')
            .context("failed to skip until opening bracket")?;

        self.idx += start + 1;

        match self.bytes[self.idx] {
            b'{' => {}
            b']' => {
                self.idx += 1;

                return Ok(());
            }
            _ => bail!("expected opening brace or closing bracket"),
        }

        let mut parentheses = memchr::memchr2_iter(b'{', b'}', &self.bytes[self.idx..]);

        // The first opening brace is already handled. We don't want to skip it
        // via index offset because all future iterator items would be affected
        // by that offset too which would make things more complicated.
        parentheses.next();

        let mut init = 0;
        let mut prev_depth = 1;
        let mut prev_idx = init;
        let mut id = None;

        for i in parentheses {
            let curr_depth = match self.bytes[self.idx + i] {
                b'{' => prev_depth + 1,
                b'}' => prev_depth - 1,
                _ => unreachable!(),
            };

            if id.is_none() && prev_depth == 1 {
                const ID: &[u8] = br#""id":"#;

                let slice = &self.bytes[self.idx + prev_idx..self.idx + i];

                if let Some(id_idx) = memmem::find(slice, ID) {
                    let n = Self::peek_u64(&slice[id_idx + ID.len()..])
                        .context("failed to peek u64")?;

                    id = Some(n);
                }
            }

            match curr_depth {
                1 => {
                    if prev_depth == 0 {
                        init = i;
                    }

                    prev_idx = i;
                }
                0 => {
                    let bytes = self.bytes.slice(self.idx + init..=self.idx + i);

                    let id = id
                        .take()
                        .ok_or_else(|| eyre!("missing id within bytes {bytes:?}"))?;

                    scores.insert(Score { bytes, id });

                    match self.bytes[self.idx + i + 1] {
                        b',' => {}
                        b']' => break,
                        _ => bail!("expected comma or closing bracket"),
                    }
                }
                _ => {}
            }

            prev_depth = curr_depth;
        }

        Ok(())
    }

    fn skip_whitespace_until(bytes: &[u8], until: fn(u8) -> bool) -> Result<usize> {
        bytes
            .iter()
            .enumerate()
            .try_fold((), |_, (idx, &byte)| match byte {
                b' ' => ControlFlow::Continue(()),
                _ if until(byte) => ControlFlow::Break(Ok(idx)),
                _ => ControlFlow::Break(Err(eyre!("unexpected character `{}`", byte as char))),
            })
            .break_value()
            .ok_or(eyre!("`until` condition never met"))?
    }

    fn peek_u64(bytes: &[u8]) -> Result<u64> {
        let start = Self::skip_whitespace_until(bytes, |byte| byte.is_ascii_digit())
            .context(eyre!("failed to skip until digit"))?;

        let n = bytes[start..]
            .iter()
            .copied()
            .take_while(u8::is_ascii_digit)
            .fold(0, |n, byte| n * 10 + (byte & 0xF) as u64);

        Ok(n)
    }
}

#[cfg_attr(test, derive(Debug))]
pub struct Score {
    bytes: Bytes,
    pub id: u64,
}

impl Score {
    pub fn only_id(id: u64) -> Self {
        Self {
            bytes: Bytes::new(),
            id,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn as_message(&self) -> Message {
        Message::Binary(self.bytes.clone())
    }
}

impl PartialEq for Score {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Score {}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCORES: &[u8] = br#"{"scores": [{"id": 123}, {"id":456, "user": {"id": 2}}, {"user": {"id":2}, "id": 789}], "cursor": {"a":true, "b": false, "c": null, "d": 0123, "e": [true, false, null, 0123, [], {}, "abc"], "f": {}, "g": "abc"}, "cursor_string": "abc"}"#;

    impl PartialEq<(&[u8], u64)> for &Score {
        fn eq(&self, (bytes, id): &(&[u8], u64)) -> bool {
            self.bytes == bytes && self.id == *id
        }
    }

    #[test]
    fn deserialize() {
        let mut scores = BTreeSet::new();

        ScoresDeserializer::new(SCORES.into())
            .deserialize(&mut scores)
            .unwrap();

        let mut iter = scores.iter();

        assert_eq!(iter.next().unwrap(), (br#"{"id": 123}"#.as_slice(), 123));
        assert_eq!(
            iter.next().unwrap(),
            (br#"{"id":456, "user": {"id": 2}}"#.as_slice(), 456)
        );
        assert_eq!(
            iter.next().unwrap(),
            (br#"{"user": {"id":2}, "id": 789}"#.as_slice(), 789)
        );
        assert!(iter.next().is_none());
    }
}
