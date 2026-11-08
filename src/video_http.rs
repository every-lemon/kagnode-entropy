//! Public-address HTTP(S) transport, independent of the decoder. There is no
//! proxy inheritance, cookie jar, authorization header or decoder network access.
use crate::video::{EncodedInput, Operation, VideoConfig, VideoError, VideoTransport};
use reqwest::{
    Url,
    dns::{Addrs, Name, Resolve, Resolving},
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => public_v4(v),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v.segments();
            // Global unicast only. Exclude documentation/transition ranges which
            // can embed another destination or cannot be a public source.
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && s[0] != 0x3ffe
                && !(s[0] == 0x3fff && s[1] < 0x1000)
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
        }
    }
}
fn public_v4(v: Ipv4Addr) -> bool {
    let a = v.octets();
    !(v.is_private()
        || v.is_loopback()
        || v.is_link_local()
        || v.is_broadcast()
        || v.is_unspecified()
        || v.is_documentation()
        || a[0] == 0
        || a[0] >= 224
        || (a[0] == 100 && (64..=127).contains(&a[1]))
        || (a[0] == 192 && a[1] == 0 && a[2] == 0)
        || (a[0] == 192 && a[1] == 88 && a[2] == 99)
        || (a[0] == 198 && (18..=19).contains(&a[1])))
}
pub fn validate_url(value: &str) -> Result<Url, VideoError> {
    if value.len() > 4096 {
        return Err(VideoError::Configuration);
    }
    let url = Url::parse(value).map_err(|_| VideoError::Configuration)?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.port_or_known_default(), Some(80 | 443))
    {
        return Err(VideoError::Configuration);
    }
    let host = url
        .host_str()
        .ok_or(VideoError::Configuration)?
        .trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err(VideoError::Configuration);
    }
    if let Some(host) = url.host() {
        match host {
            url::Host::Ipv4(v) if !public_address(IpAddr::V4(v)) => {
                return Err(VideoError::Configuration);
            }
            url::Host::Ipv6(v) if !public_address(IpAddr::V6(v)) => {
                return Err(VideoError::Configuration);
            }
            _ => {}
        }
    }
    Ok(url)
}
pub fn checked_addresses(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, VideoError> {
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses.iter().any(|a| !public_address(a.ip()))
    {
        return Err(VideoError::Network);
    }
    Ok(addresses)
}
struct PublicResolver;
impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .take(33)
                .collect();
            let addresses = checked_addresses(addresses)?;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
pub struct HttpTransport;
impl VideoTransport for HttpTransport {
    fn open<'a>(&'a self, config: &'a VideoConfig) -> Operation<'a, Box<dyn EncodedInput>> {
        Box::pin(async move {
            let url = validate_url(&config.url)?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .tls_sslkeylogfile(false)
                .http1_only()
                .referer(false)
                .pool_max_idle_per_host(0)
                .dns_resolver(Arc::new(PublicResolver))
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(config.frame_timeout)
                .timeout(config.session_lifetime + config.frame_timeout)
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if validate_redirect(attempt.previous(), attempt.url()).is_err() {
                        attempt.error("Video redirect rejected")
                    } else {
                        attempt.follow()
                    }
                }))
                .user_agent("KagNode-Entropy/0.1 source-preview")
                .build()
                .map_err(|_| VideoError::Configuration)?;
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|_| VideoError::Network)?;
            let media = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            validate_response(
                response.status().as_u16(),
                response.content_length(),
                media,
                config.max_input_bytes,
            )?;
            Ok(Box::new(HttpInput { response }) as Box<dyn EncodedInput>)
        })
    }
}
struct HttpInput {
    response: reqwest::Response,
}
impl EncodedInput for HttpInput {
    fn next_chunk(&mut self) -> Operation<'_, Option<bytes::Bytes>> {
        Box::pin(async move { self.response.chunk().await.map_err(|_| VideoError::Network) })
    }
}

pub fn validate_response(
    status: u16,
    length: Option<u64>,
    content_type: &str,
    max_bytes: u64,
) -> Result<(), VideoError> {
    if !(200..300).contains(&status) {
        return Err(VideoError::Network);
    }
    if length.is_some_and(|n| n > max_bytes) {
        return Err(VideoError::InputLimit);
    }
    let media = content_type.split(';').next().unwrap_or("").trim();
    if !media.starts_with("video/")
        && !matches!(media, "application/octet-stream" | "application/ogg")
    {
        return Err(VideoError::Configuration);
    }
    Ok(())
}

pub fn validate_redirect(previous: &[Url], target: &Url) -> Result<(), VideoError> {
    if previous.len() >= 3
        || previous.last().is_some_and(|u| u.scheme() == "https") && target.scheme() != "https"
    {
        return Err(VideoError::Configuration);
    }
    validate_url(target.as_str()).map(|_| ())
}
