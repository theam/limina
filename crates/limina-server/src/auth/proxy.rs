//! Which reverse proxies may tell the runtime the original scheme and client address.
//!
//! The Python runtime ran under uvicorn, which by default believes `X-Forwarded-Proto` and
//! `X-Forwarded-For` from `127.0.0.1` only (`FORWARDED_ALLOW_IPS`). The same rule applies here:
//! the scheme decides the request origin (redirect URI, the Origin check, `Secure` cookies) and
//! the client address keys the per-client failure limit. Headers from any other peer are ignored.

use std::net::IpAddr;

use axum::http::Uri;
use axum::http::header::HeaderMap;

use super::config::AuthConfigError;

/// `FORWARDED_ALLOW_IPS`: `*`, or a comma-separated list of proxy IP addresses (default
/// `127.0.0.1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedProxies {
    Any,
    Addresses(Vec<IpAddr>),
}

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::Addresses(vec![IpAddr::from([127, 0, 0, 1])])
    }
}

impl TrustedProxies {
    pub fn parse(value: &str) -> Result<Self, AuthConfigError> {
        let items: Vec<&str> =
            value.split(',').map(str::trim).filter(|item| !item.is_empty()).collect();
        if items.contains(&"*") {
            return Ok(Self::Any);
        }
        items
            .into_iter()
            .map(|item| {
                item.parse().map_err(|_| {
                    AuthConfigError::new(format!(
                        "FORWARDED_ALLOW_IPS must list IP addresses or *, not {item:?}."
                    ))
                })
            })
            .collect::<Result<_, _>>()
            .map(Self::Addresses)
    }

    pub fn trusts(&self, address: IpAddr) -> bool {
        match self {
            Self::Any => true,
            Self::Addresses(addresses) => addresses.contains(&address),
        }
    }

    /// The scheme the client used and its address, from the peer and, when the peer is a trusted
    /// proxy, its forwarding headers. For `X-Forwarded-For`, the rightmost untrusted address is the
    /// client (or the leftmost when all are trusted), as uvicorn decides.
    pub fn resolve(
        &self,
        peer: Option<IpAddr>,
        uri: &Uri,
        headers: &HeaderMap,
    ) -> (String, Option<IpAddr>) {
        let mut scheme = uri.scheme_str().unwrap_or("http").to_owned();
        let Some(peer) = peer.filter(|peer| self.trusts(*peer)) else {
            return (scheme, peer);
        };
        let header = |name: &str| {
            headers.get(name).map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        };
        if let Some(proto) = header("x-forwarded-proto") {
            match proto.trim() {
                "http" | "ws" => "http".clone_into(&mut scheme),
                "https" | "wss" => "https".clone_into(&mut scheme),
                _ => {}
            }
        }
        let client = header("x-forwarded-for")
            .and_then(|chain| {
                let hops: Vec<IpAddr> =
                    chain.split(',').filter_map(|hop| hop.trim().parse().ok()).collect();
                hops.iter().rev().find(|hop| !self.trusts(**hop)).or(hops.first()).copied()
            })
            .unwrap_or(peer);
        (scheme, Some(client))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn only_trusted_proxies_set_the_scheme_and_client() {
        let uri = Uri::from_static("/v1/me");
        let local = Some(IpAddr::from([127, 0, 0, 1]));
        let remote = Some(IpAddr::from([203, 0, 113, 9]));
        let forwarded = headers(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "198.51.100.1, 198.51.100.2"),
        ]);
        let proxies = TrustedProxies::default();
        assert_eq!(
            proxies.resolve(local, &uri, &forwarded),
            ("https".to_owned(), Some(IpAddr::from([198, 51, 100, 2])))
        );
        assert_eq!(proxies.resolve(remote, &uri, &forwarded), ("http".to_owned(), remote));
        assert_eq!(proxies.resolve(None, &uri, &forwarded), ("http".to_owned(), None));
        let all = TrustedProxies::parse("*").unwrap();
        assert_eq!(all.resolve(remote, &uri, &forwarded).0, "https");
        let chain = TrustedProxies::parse("127.0.0.1, 198.51.100.2").unwrap();
        assert_eq!(chain.resolve(local, &uri, &forwarded).1, Some(IpAddr::from([198, 51, 100, 1])));
        assert!(TrustedProxies::parse("not-an-ip").is_err());
    }
}
