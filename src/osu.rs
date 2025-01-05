use bytes::Bytes;
use eyre::Result;
use memchr::Memchr2;

use crate::config::OsuConfig;

pub struct Osu {}

static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Osu {
    pub fn new(_config: OsuConfig) -> Self {
        Self {}
    }

    pub async fn fetch_scores(&self) -> Result<Scores> {
        let scores = format!(
            r#"[{{"id": {}, "user": {{"id": 2}}}}]"#,
            COUNT.fetch_add(1, std::sync::atomic::Ordering::Release)
        );

        Ok(Scores(scores.into_bytes().into()))
    }
}

pub struct Scores(Bytes);

impl Scores {
    pub fn iter(&self) -> ScoresIter<'_> {
        ScoresIter {
            bytes: &self.0,
            inner: Memchr2::new(b'{', b'}', &self.0),
        }
    }

    pub fn last(&self) -> Option<Score<'_>> {
        self.iter().last()
    }
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct Score<'a> {
    bytes: &'a [u8],
    pub id: Option<ScoreId>,
}

impl Score<'_> {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.bytes).unwrap()
    }
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct ScoreId(pub u64);

impl ScoreId {
    pub fn try_parse(bytes: &[u8]) -> Option<Self> {
        let mut id = 0;

        for byte in bytes {
            match byte {
                b'0'..=b'9' => {
                    id *= 10;
                    id += (byte & 0xf) as u64;
                }
                b' ' => {}
                b',' | b'}' => break,
                _ => return None,
            }
        }

        Some(Self(id))
    }
}

pub struct ScoresIter<'a> {
    bytes: &'a [u8],
    inner: Memchr2<'a>,
}

impl<'a> Iterator for ScoresIter<'a> {
    type Item = Score<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.inner.next()?;
        debug_assert_eq!(self.bytes[start], b'{');

        let mut prev = 1;
        let mut prev_idx = start;
        let mut id = None;

        loop {
            let idx = self.inner.next()?;

            let curr = match self.bytes[idx] {
                b'{' => prev + 1,
                b'}' => prev - 1,
                _ => unreachable!(),
            };

            if id.is_none() && prev == 1 {
                Self::parse_id(&mut id, &self.bytes[prev_idx..idx]);
            }

            match curr {
                1 => prev_idx = idx,
                0 => {
                    let bytes = &self.bytes[start..=idx];

                    return Some(Score { bytes, id });
                }
                _ => {}
            }

            prev = curr;
        }
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl DoubleEndedIterator for ScoresIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let end = self.inner.next_back()?;
        debug_assert_eq!(self.bytes[end], b'}');

        let mut prev = 1;
        let mut prev_idx = end;
        let mut id = None;

        loop {
            let idx = self.inner.next_back()?;

            let curr = match self.bytes[idx] {
                b'}' => prev + 1,
                b'{' => prev - 1,
                _ => unreachable!(),
            };

            if id.is_none() && prev == 1 {
                Self::parse_id(&mut id, &self.bytes[idx..prev_idx]);
            }

            match curr {
                1 => prev_idx = idx,
                0 => {
                    let bytes = &self.bytes[idx..=end];

                    return Some(Score { bytes, id });
                }
                _ => {}
            }

            prev = curr;
        }
    }
}

impl ScoresIter<'_> {
    fn parse_id(id: &mut Option<ScoreId>, bytes: &[u8]) {
        const PREFIX: &[u8] = br#""id":"#;

        let Some(idx) = memchr::memmem::find(bytes, PREFIX) else {
            return;
        };

        let start = idx + PREFIX.len();

        let parsed = ScoreId::try_parse(&bytes[start..])
            .unwrap_or_else(|| panic!("invalid score id bytes: {:?}", &bytes[start..]));

        *id = Some(parsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCORES: &[u8] = br#"
[
    {"id": 123},
    {"id":456, "user": {"id": 2}},
    {"user": {"id":2}, "id": 789},
    {"user": {"id": 2}}
]"#;

    #[test]
    fn scores_iter_forward() {
        let scores = Scores(SCORES.into());
        let mut iter = scores.iter();

        assert_eq!(
            iter.next().unwrap(),
            Score {
                bytes: br#"{"id": 123}"#,
                id: Some(ScoreId(123)),
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            Score {
                bytes: br#"{"id":456, "user": {"id": 2}}"#,
                id: Some(ScoreId(456)),
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            Score {
                bytes: br#"{"user": {"id":2}, "id": 789}"#,
                id: Some(ScoreId(789)),
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            Score {
                bytes: br#"{"user": {"id": 2}}"#,
                id: None
            }
        );
    }

    #[test]
    fn scores_iter_backward() {
        let scores = Scores(SCORES.into());
        let mut iter = scores.iter();

        assert_eq!(
            iter.next_back().unwrap(),
            Score {
                bytes: br#"{"user": {"id": 2}}"#,
                id: None
            }
        );
        assert_eq!(
            iter.next_back().unwrap(),
            Score {
                bytes: br#"{"user": {"id":2}, "id": 789}"#,
                id: Some(ScoreId(789)),
            }
        );
        assert_eq!(
            iter.next_back().unwrap(),
            Score {
                bytes: br#"{"id":456, "user": {"id": 2}}"#,
                id: Some(ScoreId(456)),
            }
        );
        assert_eq!(
            iter.next_back().unwrap(),
            Score {
                bytes: br#"{"id": 123}"#,
                id: Some(ScoreId(123)),
            }
        );
    }
}
