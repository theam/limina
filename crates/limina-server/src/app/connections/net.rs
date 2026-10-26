//! Which servers connections may reach, and the HTTP client every outbound connection request
//! (MCP probes, OAuth discovery, registration, token requests) goes through.
//!
//! - **URLs** must be `https://`; plain `http://` only for loopback (`localhost`, `127.0.0.0/8`,
//!   `::1`), for development. No user information in the URL, no fragment.
//! - **Addresses**: link-local addresses (which include the cloud instance metadata service at
//!   `169.254.169.254`), other known metadata endpoints (`fd00:ec2::254`, `100.100.100.200`,
//!   `metadata.google.internal`, …), unspecified, broadcast and multicast addresses are refused,
//!   whether written in the URL or returned by DNS. A host name may only resolve to loopback when
//!   it is `localhost` itself, so DNS cannot point a public name at the runtime's own services.
//!   Private networks are allowed: MCP servers often live inside one.
//! - **Requests** time out ([`CONNECT_TIMEOUT`], [`REQUEST_TIMEOUT`]), read at most 1 MiB, and
//!   follow at most five redirects, each to a URL that passes the same rules (never from `https`
//!   to plain `http` on a public host).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::http::Response;
use futures::StreamExt as _;
use limina_core::{LiminaError, Result};
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};
use url::Url;

/// Connecting to a server.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// One whole request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The longest server URL accepted.
pub const URL_LIMIT: usize = 2048;
const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Host names of cloud instance metadata services.
const METADATA_HOSTS: [&str; 6] = [
    "metadata",
    "metadata.google.internal",
    "metadata.azure.internal",
    "metadata.oraclecloud.com",
    "instance-data",
    "instance-data.ec2.internal",
];

/// Why a URL or address is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Refused(pub &'static str);

const NOT_HTTPS: Refused =
    Refused("Use an https:// URL (plain http:// only works for localhost, for development).");
const CREDENTIALS_IN_URL: Refused = Refused(
    "Remove the user name and password from the URL; add a token to the connection instead.",
);
const FRAGMENT: Refused = Refused("The URL cannot have a #fragment.");
const NO_HOST: Refused = Refused("The URL needs a host name.");
const BLOCKED: Refused =
    Refused("Limina does not connect to link-local or cloud metadata addresses.");
const LOOPBACK_BY_NAME: Refused =
    Refused("The server's name resolves to this machine; use localhost to reach a local server.");

/// Parses and checks a server URL someone entered (see the module docs).
pub fn server_url(raw: &str) -> Result<Url> {
    let invalid = |reason: &str| {
        LiminaError::invariant(format!("That server URL can't be used: {reason}"))
            .with_code("invalid_server_url")
            .with("server_url", raw)
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > URL_LIMIT {
        return Err(invalid("enter a URL of at most 2048 characters."));
    }
    let url = Url::parse(trimmed).map_err(|_| invalid("it is not a valid URL."))?;
    check_target(&url).map_err(|refused| invalid(refused.0))?;
    Ok(url)
}

/// The rules every URL Limina requests must pass, before DNS (see the module docs).
pub fn check_target(url: &Url) -> std::result::Result<(), Refused> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CREDENTIALS_IN_URL);
    }
    if url.fragment().is_some() {
        return Err(FRAGMENT);
    }
    let host = match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if METADATA_HOSTS.contains(&name.as_str()) {
                return Err(BLOCKED);
            }
            Target::Name(name)
        }
        Some(url::Host::Ipv4(ip)) => Target::Ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Target::Ip(IpAddr::V6(ip)),
        None => return Err(NO_HOST),
    };
    if let Target::Ip(ip) = host
        && blocked(ip)
    {
        return Err(BLOCKED);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if host.is_loopback() => Ok(()),
        _ => Err(NOT_HTTPS),
    }
}

/// Resolves the URL's host the way requests will and applies the address rules, so a URL that
/// only resolves to refused addresses fails with a clear message before anything is stored or
/// handed to an engine.
pub async fn check_resolution(url: &Url) -> Result<()> {
    let refused = |reason: &str| {
        LiminaError::invariant(format!("That server URL can't be used: {reason}"))
            .with_code("invalid_server_url")
            .with("server_url", url.as_str())
    };
    check_target(url).map_err(|refused_by| refused(refused_by.0))?;
    let Some(url::Host::Domain(name)) = url.host() else { return Ok(()) };
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((name, port)))
        .await
        .map_err(|_| unreachable(url, "the name did not resolve in time"))?
        .map_err(|error| unreachable(url, &error.to_string()))?
        .collect::<Vec<_>>();
    allowed_addresses(name, addresses).map(|_| ()).map_err(|refused_by| refused(refused_by.0))
}

/// The resolved addresses a request may connect to.
fn allowed_addresses(
    name: &str,
    addresses: Vec<SocketAddr>,
) -> std::result::Result<Vec<SocketAddr>, Refused> {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    let local_name = loopback_name(&name);
    let resolved = !addresses.is_empty();
    let allowed: Vec<SocketAddr> = addresses
        .into_iter()
        .filter(|address| {
            let ip = canonical(address.ip());
            !blocked(ip) && (local_name || !ip.is_loopback())
        })
        .collect();
    match (allowed.is_empty(), resolved) {
        (false, _) => Ok(allowed),
        (true, false) => Err(Refused("The server's name does not resolve.")),
        (true, true) if local_name => Err(BLOCKED),
        (true, true) => Err(LOOPBACK_BY_NAME),
    }
}

/// The error for a server that cannot be reached.
pub fn unreachable(url: &Url, reason: &str) -> LiminaError {
    LiminaError::transport(format!(
        "Limina could not reach {}.",
        url.host_str().unwrap_or("the server")
    ))
    .with_code("connection_failed")
    .with("reason", reason)
}

enum Target {
    Name(String),
    Ip(IpAddr),
}

impl Target {
    fn is_loopback(&self) -> bool {
        match self {
            Self::Name(name) => loopback_name(name),
            Self::Ip(ip) => canonical(*ip).is_loopback(),
        }
    }
}

fn loopback_name(name: &str) -> bool {
    name == "localhost" || name.ends_with(".localhost")
}

/// IPv4-mapped IPv6 addresses are judged as the IPv4 address they carry.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// Addresses no connection may reach (loopback is decided separately).
fn blocked(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(v4) => {
            v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.octets()[0] == 0
                || v4 == Ipv4Addr::new(100, 100, 100, 200)
        }
        IpAddr::V6(v6) => {
            v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)
        }
    }
}

/// Resolves names for outbound connection requests, keeping only allowed addresses.
struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let allowed = allowed_addresses(&host, addresses)
                .map_err(|refused| Box::new(refused) as Box<dyn std::error::Error + Send + Sync>)?;
            let addresses: reqwest::dns::Addrs = Box::new(allowed.into_iter());
            Ok(addresses)
        })
    }
}

/// The HTTP client for connection requests (see the module docs). Cheap to clone.
#[derive(Clone)]
pub struct OutboundHttp {
    /// Follows checked redirects.
    follow: reqwest::Client,
    /// Returns redirects as they are (OAuth token requests and discovery).
    stop: reqwest::Client,
}

impl std::fmt::Debug for OutboundHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OutboundHttp(..)")
    }
}

impl OutboundHttp {
    pub fn new() -> Result<Self> {
        let build = |redirects: reqwest::redirect::Policy| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .redirect(redirects)
                .dns_resolver(Arc::new(GuardedResolver))
                // A proxy would decide where requests go; connections go where their URL says.
                .no_proxy()
                .user_agent(concat!("limina/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| {
                    LiminaError::transport(format!("Cannot build the HTTP client: {error}"))
                })
        };
        let checked = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error(Refused("Too many redirects."));
            }
            match check_target(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(refused) => attempt.error(refused),
            }
        });
        Ok(Self { follow: build(checked)?, stop: build(reqwest::redirect::Policy::none())? })
    }

    /// The client that follows checked redirects, for MCP requests.
    pub fn client(&self) -> &reqwest::Client {
        &self.follow
    }
}

impl OAuthHttpClient for OutboundHttp {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let OAuthHttpRequest { request, redirect_policy, timeout, .. } = request;
            let url = Url::parse(&request.uri().to_string())
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            check_target(&url).map_err(|refused| Box::new(refused) as OAuthHttpClientError)?;
            let client = match redirect_policy {
                OAuthHttpRedirectPolicy::Stop => &self.stop,
                _ => &self.follow,
            };
            let mut request = reqwest::Request::try_from(request)
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            *request.timeout_mut() =
                Some(timeout.map_or(REQUEST_TIMEOUT, |wanted| wanted.min(REQUEST_TIMEOUT)));
            let response = client
                .execute(request)
                .await
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            let mut builder = Response::builder().status(response.status());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let body = read_limited(response).await?;
            builder.body(body).map_err(|error| Box::new(error) as OAuthHttpClientError)
        })
    }
}

/// The response body, refusing more than 1 MiB.
pub async fn read_limited(
    response: reqwest::Response,
) -> std::result::Result<Vec<u8>, OAuthHttpClientError> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| Box::new(error) as OAuthHttpClientError)?;
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Box::new(Refused("The server's response is too large.")));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(raw: &str) -> String {
        server_url(raw).unwrap_err().message
    }

    #[test]
    fn server_urls_must_be_https_except_on_loopback() {
        assert!(server_url("https://mcp.example.com/mcp").is_ok());
        assert!(server_url("https://10.1.2.3/mcp").is_ok(), "private networks are allowed");
        for local in [
            "http://localhost:8080/mcp",
            "http://app.localhost/mcp",
            "http://127.0.0.1:9000/mcp",
            "http://127.9.9.9/mcp",
            "http://[::1]:9000/mcp",
        ] {
            assert!(server_url(local).is_ok(), "{local}");
        }
        assert!(refused("http://mcp.example.com/mcp").contains("https://"));
        assert!(refused("http://10.0.0.5/mcp").contains("https://"));
        assert!(refused("ftp://mcp.example.com/").contains("https://"));
        assert!(refused("not a url").contains("not a valid URL"));
        assert!(refused("").contains("2048"));
        assert!(
            refused(&format!("https://example.com/{}", "a".repeat(URL_LIMIT))).contains("2048")
        );
        assert!(refused("https://user:pass@mcp.example.com/").contains("user name"));
        assert!(refused("https://mcp.example.com/mcp#x").contains("fragment"));
        assert_eq!(server_url("http://mcp.example.com").unwrap_err().code(), "invalid_server_url");
    }

    #[test]
    fn link_local_and_metadata_addresses_are_refused() {
        for blocked in [
            "https://169.254.169.254/latest/meta-data",
            "http://169.254.1.1/mcp",
            "https://[fe80::1]/mcp",
            "https://[fd00:ec2::254]/",
            "https://[::ffff:169.254.169.254]/",
            "https://100.100.100.200/",
            "https://0.0.0.0/mcp",
            "https://[::]/mcp",
            "https://224.0.0.1/",
            "https://metadata.google.internal/computeMetadata/v1/",
            "https://Metadata.Google.Internal./",
            "https://metadata/",
        ] {
            assert!(refused(blocked).contains("metadata"), "{blocked}");
        }
    }

    #[test]
    fn dns_answers_are_filtered() {
        let at = |ip: &str| SocketAddr::new(ip.parse().unwrap(), 443);
        assert_eq!(
            allowed_addresses("mcp.example.com", vec![at("169.254.169.254"), at("203.0.113.7")]),
            Ok(vec![at("203.0.113.7")])
        );
        assert_eq!(
            allowed_addresses("evil.example.com", vec![at("127.0.0.1")]),
            Err(LOOPBACK_BY_NAME)
        );
        assert_eq!(
            allowed_addresses("evil.example.com", vec![at("::ffff:127.0.0.1")]),
            Err(LOOPBACK_BY_NAME)
        );
        assert_eq!(
            allowed_addresses("evil.example.com", vec![at("fe80::1")]),
            Err(LOOPBACK_BY_NAME)
        );
        assert_eq!(
            allowed_addresses("localhost", vec![at("127.0.0.1"), at("::1")]).unwrap().len(),
            2
        );
        assert_eq!(allowed_addresses("localhost", vec![at("169.254.0.1")]), Err(BLOCKED));
        assert!(allowed_addresses("nothing.example.com", Vec::new()).is_err());
        assert_eq!(allowed_addresses("intranet.example", vec![at("10.0.0.8")]).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn redirects_to_plain_http_are_refused() {
        // A local server that redirects to a public http:// URL.
        let app = axum::Router::new().route(
            "/start",
            axum::routing::get(|| async {
                axum::response::Redirect::temporary("http://mcp.example.com/next")
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = OutboundHttp::new().unwrap();
        let error = http.client().get(format!("http://{address}/start")).send().await.unwrap_err();
        assert!(error.is_redirect(), "{error}");
    }
}
