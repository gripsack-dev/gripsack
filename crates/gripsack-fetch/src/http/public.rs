//! Unfollowed redirects for public release pages (installer and self-update).

use super::Client;
use crate::FetchError;
use std::io;
use std::time::Duration;

impl Client {
    pub(crate) fn redirect_location(&self, url: &str) -> Result<String, FetchError> {
        let mut builder = ureq::AgentBuilder::new()
            .tls_connector(super::tls_config(&self.certificates))
            .redirects(0)
            .timeout(Duration::from_secs(30))
            .try_proxy_from_env(false);
        if let (Some(proxy), false) = (&self.proxy, self.policy.bypasses(url)) {
            builder = builder.proxy(proxy.clone());
        }
        let response = builder
            .build()
            .get(url)
            .set("User-Agent", "gripsack")
            .call()
            .map_err(|error| io::Error::other(format!("cannot reach {url}: {error}")))?;
        let location = response
            .header("location")
            .filter(|_| (300..400).contains(&response.status()))
            .ok_or_else(|| io::Error::other(format!("{url} did not redirect")))?;
        let base = url::Url::parse(url)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(base
            .join(location)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .into())
    }
}
