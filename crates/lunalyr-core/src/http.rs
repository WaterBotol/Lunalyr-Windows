//! The HTTP seam used by providers: a small async trait, a reqwest client
//! (OS TLS: SChannel on Windows, Security.framework on macOS) and a
//! fixture client for tests.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

/// Response bodies larger than this are rejected (macOS used 2 000 000).
pub const MAX_BODY_BYTES: usize = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Header names are lower-case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        HttpResponse { status, headers: Vec::new(), body: body.into() }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.to_string()));
        self
    }

    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("The request timed out.")]
    Timeout,
    #[error("Response too large")]
    TooLarge,
    #[error("Invalid service address")]
    InvalidUrl,
    #[error("Network error: {0}")]
    Network(String),
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    /// GET `url` with extra request headers. `timeout` bounds the whole
    /// request including the body.
    async fn get(&self, url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<HttpResponse, HttpError>;
}

/// Percent-encodes a query component (everything except RFC 3986
/// unreserved characters; spaces become `%20`).
///
/// Unlike Foundation's `URLComponents.queryItems`, `+` and `&` inside values
/// are encoded, so "C++" or "Simon & Garfunkel" reach the service intact.
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `base?key=value&...` in the given order.
pub fn build_url(base: &str, parameters: &[(&str, &str)]) -> String {
    if parameters.is_empty() {
        return base.to_string();
    }
    let query: Vec<String> =
        parameters.iter().map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v))).collect();
    format!("{base}?{}", query.join("&"))
}

/// reqwest-backed client. Sends gzip `Accept-Encoding` and inflates
/// transparently; caps bodies at [`MAX_BODY_BYTES`].
#[derive(Debug, Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Result<Self, HttpError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        Ok(ReqwestClient { client })
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get(&self, url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<HttpResponse, HttpError> {
        let parsed = reqwest::Url::parse(url).map_err(|_| HttpError::InvalidUrl)?;
        let mut request = self.client.get(parsed).timeout(timeout);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let map_error = |e: reqwest::Error| {
            if e.is_timeout() {
                return HttpError::Timeout;
            }
            // reqwest's own message is generic ("error sending request");
            // the causes say what went wrong (DNS, TLS certificate, ...).
            let e = e.without_url();
            let mut message = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(cause) = source {
                let text = cause.to_string();
                if !message.contains(&text) {
                    message.push_str(": ");
                    message.push_str(&text);
                }
                source = cause.source();
            }
            HttpError::Network(message)
        };
        let mut response = request.send().await.map_err(map_error)?;
        let status = response.status().as_u16();
        let response_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_ascii_lowercase(), v.to_string())))
            .collect();
        if response.content_length().is_some_and(|n| n > MAX_BODY_BYTES as u64) {
            return Err(HttpError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_error)? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(HttpError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse { status, headers: response_headers, body })
    }
}

/// One request seen by [`FixtureHttp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
}

struct Route {
    pattern: String,
    delay: Duration,
    /// Served in order; the last one repeats.
    responses: VecDeque<Result<HttpResponse, HttpError>>,
}

/// Test client: maps URL substrings to canned responses (the longest
/// matching pattern wins; unmatched URLs get 404) and records requests.
#[derive(Default)]
pub struct FixtureHttp {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl FixtureHttp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Serves `status` + `body` for URLs containing `pattern`.
    pub fn route(self, pattern: &str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.route_response(pattern, HttpResponse::new(status, body))
    }

    pub fn route_response(self, pattern: &str, response: HttpResponse) -> Self {
        self.route_sequence(pattern, vec![Ok(response)])
    }

    /// Serves the responses in order for successive matching requests; the
    /// last one repeats.
    pub fn route_sequence(self, pattern: &str, responses: Vec<Result<HttpResponse, HttpError>>) -> Self {
        self.routes.lock().unwrap().push(Route {
            pattern: pattern.to_string(),
            delay: Duration::ZERO,
            responses: responses.into(),
        });
        self
    }

    pub fn route_error(self, pattern: &str, error: HttpError) -> Self {
        self.route_sequence(pattern, vec![Err(error)])
    }

    /// Like [`route`](Self::route) but answers after `delay` (tokio time,
    /// so paused-clock tests run instantly).
    pub fn route_delayed(self, pattern: &str, delay: Duration, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.routes.lock().unwrap().push(Route {
            pattern: pattern.to_string(),
            delay,
            responses: vec![Ok(HttpResponse::new(status, body))].into(),
        });
        self
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn urls(&self) -> Vec<String> {
        self.requests().into_iter().map(|r| r.url).collect()
    }

    /// Number of requests whose URL contains `pattern`.
    pub fn count(&self, pattern: &str) -> usize {
        self.requests.lock().unwrap().iter().filter(|r| r.url.contains(pattern)).count()
    }
}

#[async_trait]
impl HttpClient for FixtureHttp {
    async fn get(&self, url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<HttpResponse, HttpError> {
        self.requests.lock().unwrap().push(RecordedRequest {
            url: url.to_string(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            timeout,
        });
        let (delay, response) = {
            let mut routes = self.routes.lock().unwrap();
            let best = routes.iter_mut().filter(|r| url.contains(&r.pattern)).max_by_key(|r| r.pattern.len());
            match best {
                None => (Duration::ZERO, Ok(HttpResponse::new(404, Vec::new()))),
                Some(route) => {
                    let response = if route.responses.len() > 1 {
                        route.responses.pop_front().unwrap()
                    } else {
                        route.responses.front().cloned().unwrap_or_else(|| Ok(HttpResponse::new(404, Vec::new())))
                    };
                    (route.delay, response)
                }
            }
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(encode_component("C++ & Me/You?"), "C%2B%2B%20%26%20Me%2FYou%3F");
        assert_eq!(encode_component("夜"), "%E5%A4%9C");
        assert_eq!(build_url("https://x/api", &[("a", "1 2"), ("b", "")]), "https://x/api?a=1%202&b=");
        assert_eq!(build_url("https://x/api", &[]), "https://x/api");
    }

    #[tokio::test]
    async fn fixture_routes_sequences_and_records() {
        let http = FixtureHttp::new()
            .route("example.com/a", 200, "short")
            .route("example.com/a/long", 201, "long")
            .route_sequence(
                "example.com/seq",
                vec![Ok(HttpResponse::new(429, "").with_header("Retry-After", "1")), Ok(HttpResponse::new(200, "ok"))],
            );
        let t = Duration::from_secs(1);
        assert_eq!(http.get("https://example.com/a?x", &[], t).await.unwrap().body, b"short");
        assert_eq!(http.get("https://example.com/a/long", &[("User-Agent", "x")], t).await.unwrap().status, 201);
        let first = http.get("https://example.com/seq", &[], t).await.unwrap();
        assert_eq!((first.status, first.header("retry-after")), (429, Some("1")));
        assert_eq!(http.get("https://example.com/seq", &[], t).await.unwrap().status, 200);
        assert_eq!(http.get("https://example.com/seq", &[], t).await.unwrap().status, 200, "last repeats");
        assert_eq!(http.get("https://elsewhere.org", &[], t).await.unwrap().status, 404);
        assert_eq!(http.count("example.com/seq"), 3);
        assert_eq!(http.requests()[1].headers, vec![("User-Agent".to_string(), "x".to_string())]);
    }

    #[test]
    fn reqwest_client_builds() {
        assert!(ReqwestClient::new().is_ok());
    }
}
