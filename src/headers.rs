use std::{net::Ipv4Addr, str::FromStr as _};

use headers::{Header, HeaderName, HeaderValue};

#[derive(Clone, Debug)]
pub struct CloudflareConnectingIp(pub Ipv4Addr);

static NAME: HeaderName = HeaderName::from_static("cf-connecting-ip");

impl Header for CloudflareConnectingIp {
    fn name() -> &'static HeaderName {
        &NAME
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, headers::Error>
    where
        I: Iterator<Item = &'i HeaderValue>,
    {
        let ip = values
            .next()
            .and_then(|ip| ip.to_str().ok())
            .and_then(|ip| Ipv4Addr::from_str(ip).ok())
            .ok_or_else(headers::Error::invalid)?;

        Ok(Self(ip))
    }

    fn encode<E>(&self, values: &mut E)
    where
        E: Extend<HeaderValue>,
    {
        let s = self.0.to_string();
        if let Ok(value) = HeaderValue::from_str(s.as_str()) {
            values.extend(std::iter::once(value));
        }
    }
}
