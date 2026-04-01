use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

pub struct GitHubClient {
    api_base: String,
    http: reqwest::blocking::Client,
}

impl GitHubClient {
    pub fn new(api_base: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            http: reqwest::blocking::Client::new(),
        }
    }

    pub fn latest_release(&self, repo: &str, channel: &str) -> Result<Release, String> {
        let response = self
            .http
            .get(format!("{}/repos/{repo}/releases", self.api_base))
            .header(reqwest::header::USER_AGENT, "hive")
            .send()
            .map_err(|error| format!("failed to fetch GitHub releases for `{repo}`: {error}"))?
            .error_for_status()
            .map_err(|error| format!("failed to fetch GitHub releases for `{repo}`: {error}"))?;
        let body = response
            .text()
            .map_err(|error| format!("failed to read GitHub releases for `{repo}`: {error}"))?;
        let releases: Vec<Release> = serde_json::from_str(&body)
            .map_err(|error| format!("failed to decode GitHub releases for `{repo}`: {error}"))?;

        releases
            .into_iter()
            .filter(|release| !release.draft)
            .find(|release| channel == "nightly" || !release.prerelease)
            .ok_or_else(|| {
                format!("no qualifying GitHub release found for `{repo}` channel `{channel}`")
            })
    }

    pub fn download_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        self.http
            .get(url)
            .header(reqwest::header::USER_AGENT, "hive")
            .send()
            .map_err(|error| format!("failed to download {url}: {error}"))?
            .error_for_status()
            .map_err(|error| format!("failed to download {url}: {error}"))?
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|error| format!("failed to read {url}: {error}"))
    }
}
