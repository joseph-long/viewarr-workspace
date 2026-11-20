//! Where a camera's feeds are, from the URL of its data feed.
//!
//! A sup stream server serves each shmim at `/ws/viewcam/<name>`, and lists
//! the streams at `/ws/viewcam/` with each one's dark, saturation mask and
//! saturation level from its rtimv config (sup/stream_catalog.py). So the
//! data feed's URL is enough to find the rest, as sup's camera page does.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// A data feed's URL, split into the directory its siblings share and its
/// stream's name.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedUrl {
    /// Up to and including the last `/`, e.g. `wss://host/ws/viewcam/`.
    pub base: String,
    pub name: String,
}

impl FeedUrl {
    pub fn parse(url: &str) -> Result<Self> {
        if !(url.starts_with("ws://") || url.starts_with("wss://")) {
            bail!("{url:?} is not a ws:// or wss:// URL");
        }
        let cut = url.rfind('/').map_or(0, |i| i + 1);
        let (base, name) = url.split_at(cut);
        if name.is_empty() || base.ends_with("//") {
            bail!("{url:?} doesn't name a stream (expected e.g. wss://host/ws/viewcam/camwfs)");
        }
        Ok(Self {
            base: base.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The URL of another stream from the same server.
    pub fn sibling(&self, name: &str) -> String {
        format!("{}{name}", self.base)
    }

    /// The catalog's URL: the feeds' directory, over HTTP.
    pub fn catalog(&self) -> String {
        let base = &self.base;
        match base.strip_prefix("wss://") {
            Some(rest) => format!("https://{rest}"),
            None => format!("http://{}", base.trim_start_matches("ws://")),
        }
    }
}

/// A camera's optional images, as its rtimv config names them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Companions {
    pub dark: Option<String>,
    pub saturation: Option<String>,
    pub saturation_level: Option<f64>,
}

#[derive(Deserialize)]
struct Catalog {
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    name: String,
    dark: Option<String>,
    sat_mask: Option<String>,
    sat_level: Option<f64>,
}

/// Look `name` up in a catalog's JSON. A stream the catalog doesn't list,
/// or lists without a config, has no companions.
pub fn companions_in(json: &str, name: &str) -> Result<Companions> {
    let catalog: Catalog = serde_json::from_str(json).context("reading the stream catalog")?;
    Ok(catalog
        .streams
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| Companions {
            dark: s.dark,
            saturation: s.sat_mask,
            saturation_level: s.sat_level,
        })
        .unwrap_or_default())
}

/// Fetch the catalog from the server behind `url` and look its stream up.
pub fn fetch_companions(url: &FeedUrl) -> Result<Companions> {
    let catalog = url.catalog();
    let json = ureq::get(&catalog)
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .with_context(|| format!("fetching the stream catalog from {catalog}"))?;
    companions_in(&json, &url.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feed_url_gives_its_siblings_and_catalog() {
        let url = FeedUrl::parse("wss://exao1.magao-x.org/ws/viewcam/camsci2").unwrap();
        assert_eq!(url.name, "camsci2");
        assert_eq!(
            url.sibling("camsci2_dark"),
            "wss://exao1.magao-x.org/ws/viewcam/camsci2_dark"
        );
        assert_eq!(url.catalog(), "https://exao1.magao-x.org/ws/viewcam/");
        let local = FeedUrl::parse("ws://127.0.0.1:9192/ws/viewcam/camsim").unwrap();
        assert_eq!(local.catalog(), "http://127.0.0.1:9192/ws/viewcam/");
    }

    #[test]
    fn urls_without_a_stream_are_refused() {
        assert!(FeedUrl::parse("https://host/ws/viewcam/camwfs").is_err());
        assert!(FeedUrl::parse("wss://host/ws/viewcam/").is_err());
        assert!(FeedUrl::parse("wss://host").is_err());
    }

    #[test]
    fn companions_come_from_the_streams_entry() {
        let json = r#"{"streams": [
            {"name": "camwfs", "dark": "camwfs_dark", "sat_mask": null, "sat_level": 16383, "config": "x"},
            {"name": "acc000_psds", "dark": null, "sat_mask": null, "sat_level": null, "config": null}
        ]}"#;
        assert_eq!(
            companions_in(json, "camwfs").unwrap(),
            Companions {
                dark: Some("camwfs_dark".to_owned()),
                saturation: None,
                saturation_level: Some(16383.0),
            }
        );
        assert_eq!(companions_in(json, "unlisted").unwrap(), Companions::default());
    }
}
