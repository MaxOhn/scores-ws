use std::{borrow::Cow, cmp, time::Duration};

use bytes::Bytes;
use eyre::{Context as _, Result};
use http_body_util::{BodyExt, Full};
use hyper::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT},
    Request, StatusCode,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Builder, Client},
    rt::TokioExecutor,
};
use memchr::memmem;

use crate::config::OsuConfig;

use super::{authorization::Authorization, Scores, ScoresDeserializer};

const MY_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const APPLICATION_JSON: &str = "application/json";
const APPLICATION_URL_ENCODED: &str = "application/x-www-form-urlencoded";

type Body = Full<Bytes>;

pub struct Osu {
    config: OsuConfig,
    authorization: Authorization,
    client: Client<HttpsConnector<HttpConnector>, Body>,
}

impl Osu {
    pub fn new(config: OsuConfig) -> Result<Self> {
        #[cfg(feature = "ring")]
        let crypto_provider = rustls::crypto::ring::default_provider();
        #[cfg(all(feature = "aws", not(feature = "ring")))]
        let crypto_provider = rustls::crypto::aws_lc_rs::default_provider();
        #[cfg(not(any(feature = "ring", feature = "aws")))]
        let crypto_provider = rustls::crypto::CryptoProvider::get_default()
            .expect("No default crypto provider installed or configured via crate features")
            .clone();

        let https = HttpsConnectorBuilder::new()
            .with_provider_and_webpki_roots(crypto_provider)
            .context("Failed to configure https connector")?
            .https_only()
            .enable_http2()
            .build();

        let client = Builder::new(TokioExecutor::new())
            .http2_only(true)
            .build(https);

        Ok(Self {
            config,
            client,
            authorization: Authorization::default(),
        })
    }

    async fn fetch_response(&self, req: Request<Body>) -> Result<(Bytes, StatusCode)> {
        let response = self
            .client
            .request(req)
            .await
            .context("Failed to send request")?;

        let (parts, incoming) = response.into_parts();

        let bytes = incoming
            .collect()
            .await
            .context("Failed to collect bytes")?
            .to_bytes();

        Ok((bytes, parts.status))
    }

    async fn reauthorize(&self) -> Result<()> {
        const URL: &str = "https://osu.ppy.sh/oauth/token";

        info!("Re-authorizing...");

        let OsuConfig {
            client_id,
            client_secret,
            ruleset: _,
        } = &self.config;

        let body = format!(
            "client_id={client_id}&client_secret={client_secret}\
            &grant_type=client_credentials&scope=public"
        );

        let req = Request::post(URL)
            .header(USER_AGENT, MY_USER_AGENT)
            .header(ACCEPT, APPLICATION_JSON)
            .header(CONTENT_TYPE, APPLICATION_URL_ENCODED)
            .header(CONTENT_LENGTH, body.len())
            .body(Full::from(body))
            .context("Failed to create token request")?;

        let (bytes, status_code) = self
            .fetch_response(req)
            .await
            .context("Failed to fetch response")?;

        match status_code {
            StatusCode::OK => self
                .authorization
                .parse(&bytes)
                .context("Failed to parse authorization"),
            StatusCode::UNAUTHORIZED => {
                bail!(
                    "Received 401 error while authorizing, make sure your \
                    client id and secret are valid: {bytes:?}"
                )
            }
            StatusCode::TOO_MANY_REQUESTS => {
                bail!("Received 429 error, try reducing your interval: {bytes:?}")
            }
            StatusCode::SERVICE_UNAVAILABLE => {
                bail!("Received 503 error, osu! servers likely temporarily down: {bytes:?}")
            }
            _ => bail!("Status code: {status_code}, Response: {bytes:?}"),
        }
    }

    pub async fn fetch_scores(&self, scores: &mut Scores, cursor_id: Option<u64>) -> FetchResult {
        const URL: &str = "https://osu.ppy.sh/api/v2/scores";

        async fn fetch_inner(
            osu: &Osu,
            scores: &mut Scores,
            just_authorized: bool,
            cursor_id: Option<u64>,
        ) -> Result<FetchResult> {
            let mut url = Cow::Borrowed(URL);

            if let Some(ruleset) = osu.config.ruleset.as_deref() {
                let url = url.to_mut();
                url.push_str("?ruleset=");
                url.push_str(ruleset);
            }

            if let Some(cursor_id) = cursor_id {
                let is_without_query = matches!(url, Cow::Borrowed(_));
                let url = url.to_mut();

                if is_without_query {
                    url.push('?');
                } else {
                    url.push('&');
                }

                url.push_str("cursor[id]=");
                url.push_str(itoa::Buffer::new().format(cursor_id));
            }

            let req = Request::get(url.as_ref())
                .header(USER_AGENT, MY_USER_AGENT)
                // doesn't seem to affect the response data format
                // .header("x-api-version", 0_usize)
                .header(ACCEPT, APPLICATION_JSON)
                .header(AUTHORIZATION, osu.authorization.as_str())
                .header(CONTENT_LENGTH, 0_usize)
                .body(Full::default())
                .context("Failed to create request")?;

            let (bytes, status_code) = osu
                .fetch_response(req)
                .await
                .context("Failed to fetch response")?;

            match status_code {
                StatusCode::OK => {
                    ScoresDeserializer::new(bytes).deserialize(scores)?;

                    Ok(FetchResult::Ok)
                }
                StatusCode::UNAUTHORIZED => {
                    if just_authorized {
                        bail!("Received 401 error after authorizing: {bytes:?}");
                    }

                    osu.reauthorize().await.context("Failed to re-authorize")?;

                    return Box::pin(fetch_inner(osu, scores, true, cursor_id)).await;
                }
                StatusCode::UNPROCESSABLE_ENTITY
                    if memmem::rfind(&bytes, br#""error":"cursor is too old""#).is_some() =>
                {
                    if let Some(cursor_id) = cursor_id {
                        warn!("Score id {cursor_id} too old to fetch from");
                    } else {
                        debug!("\"cursor too old\" without a cursor id");
                    }

                    Ok(FetchResult::CursorTooOld)
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    bail!("Received 429 error, try reducing your interval: {bytes:?}")
                }
                StatusCode::SERVICE_UNAVAILABLE => {
                    bail!("Received 503 error, osu! servers likely temporarily down: {bytes:?}")
                }
                _ => bail!("Status code: {status_code}, Response: {bytes:?}"),
            }
        }

        info!(?cursor_id, "Fetching scores...");

        let mut backoff = 2;

        loop {
            let fetch_fut = fetch_inner(self, scores, false, cursor_id);

            match tokio::time::timeout(Duration::from_secs(10), fetch_fut).await {
                Ok(Ok(res)) => return res,
                Ok(Err(err)) => error!(?err, "Failed to fetch scores"),
                Err(_) => error!("Timeout while awaiting scores"),
            }

            info!("Retrying in {backoff}s...");
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = cmp::min(120, backoff * 2);
        }
    }
}

#[derive(Default)]
pub enum FetchResult {
    #[default]
    Ok,
    CursorTooOld,
}
