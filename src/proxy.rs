#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedValue {
    env_name: &'static str,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ProxySettings {
    http_proxy: Option<ResolvedValue>,
    https_proxy: Option<ResolvedValue>,
    all_proxy: Option<ResolvedValue>,
    no_proxy: Option<ResolvedValue>,
}

pub fn build_http_client() -> Result<reqwest::blocking::Client, String> {
    let settings = resolve_proxy_settings_from(|name| std::env::var(name).ok());
    let https_proxy = effective_https_proxy(&settings);
    let no_proxy = settings
        .no_proxy
        .as_ref()
        .and_then(|resolved| reqwest::NoProxy::from_string(&resolved.value));
    let mut builder = reqwest::blocking::Client::builder().no_proxy();

    if let Some(proxy) = settings.http_proxy {
        builder = builder.proxy(
            reqwest::Proxy::http(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy.clone()),
        );
    }

    if let Some(proxy) = https_proxy {
        builder = builder.proxy(
            reqwest::Proxy::https(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy.clone()),
        );
    }

    if let Some(proxy) = settings.all_proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(&proxy.value)
                .map_err(|_| format!("invalid proxy URL in {}", proxy.env_name))?
                .no_proxy(no_proxy),
        );
    }

    builder
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))
}

fn resolve_proxy_settings_from<F>(mut get: F) -> ProxySettings
where
    F: FnMut(&str) -> Option<String>,
{
    ProxySettings {
        http_proxy: resolve_value(&mut get, "HIVE_HTTP_PROXY", "HTTP_PROXY", "http_proxy"),
        https_proxy: resolve_value(&mut get, "HIVE_HTTPS_PROXY", "HTTPS_PROXY", "https_proxy"),
        all_proxy: resolve_value(&mut get, "HIVE_ALL_PROXY", "ALL_PROXY", "all_proxy"),
        no_proxy: resolve_value(&mut get, "HIVE_NO_PROXY", "NO_PROXY", "no_proxy"),
    }
}

fn effective_https_proxy(settings: &ProxySettings) -> Option<ResolvedValue> {
    settings
        .https_proxy
        .clone()
        .or_else(|| settings.http_proxy.clone())
}

fn resolve_value<F>(
    get: &mut F,
    hive_name: &'static str,
    upper_name: &'static str,
    lower_name: &'static str,
) -> Option<ResolvedValue>
where
    F: FnMut(&str) -> Option<String>,
{
    for name in [hive_name, upper_name, lower_name] {
        if let Some(value) = get(name) {
            return Some(ResolvedValue {
                env_name: name,
                value,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{ProxySettings, ResolvedValue, effective_https_proxy, resolve_proxy_settings_from};
    use std::collections::HashMap;

    #[test]
    fn hive_proxy_overrides_standard_proxy() {
        let env = HashMap::from([
            ("HIVE_HTTP_PROXY", "http://hive-proxy:8080"),
            ("HTTP_PROXY", "http://upper-proxy:8080"),
            ("http_proxy", "http://lower-proxy:8080"),
        ]);

        let settings =
            resolve_proxy_settings_from(|name| env.get(name).map(|value| value.to_string()));

        assert_eq!(
            settings.http_proxy,
            Some(ResolvedValue {
                env_name: "HIVE_HTTP_PROXY",
                value: "http://hive-proxy:8080".to_string(),
            })
        );
    }

    #[test]
    fn uppercase_standard_proxy_overrides_lowercase() {
        let env = HashMap::from([
            ("HTTPS_PROXY", "http://upper-proxy:8080"),
            ("https_proxy", "http://lower-proxy:8080"),
        ]);

        let settings =
            resolve_proxy_settings_from(|name| env.get(name).map(|value| value.to_string()));

        assert_eq!(
            settings.https_proxy,
            Some(ResolvedValue {
                env_name: "HTTPS_PROXY",
                value: "http://upper-proxy:8080".to_string(),
            })
        );
    }

    #[test]
    fn resolves_each_slot_independently() {
        let env = HashMap::from([
            ("HIVE_HTTP_PROXY", "http://hive-http:8080"),
            ("HTTPS_PROXY", "http://std-https:8080"),
            ("all_proxy", "http://lower-all:8080"),
            ("NO_PROXY", "localhost,127.0.0.1"),
        ]);

        let settings =
            resolve_proxy_settings_from(|name| env.get(name).map(|value| value.to_string()));

        assert_eq!(
            settings,
            ProxySettings {
                http_proxy: Some(ResolvedValue {
                    env_name: "HIVE_HTTP_PROXY",
                    value: "http://hive-http:8080".to_string(),
                }),
                https_proxy: Some(ResolvedValue {
                    env_name: "HTTPS_PROXY",
                    value: "http://std-https:8080".to_string(),
                }),
                all_proxy: Some(ResolvedValue {
                    env_name: "all_proxy",
                    value: "http://lower-all:8080".to_string(),
                }),
                no_proxy: Some(ResolvedValue {
                    env_name: "NO_PROXY",
                    value: "localhost,127.0.0.1".to_string(),
                }),
            }
        );
    }

    #[test]
    fn https_proxy_falls_back_to_http_proxy_when_https_unset() {
        let env = HashMap::from([("HTTP_PROXY", "http://upper-http:8080")]);

        let settings =
            resolve_proxy_settings_from(|name| env.get(name).map(|value| value.to_string()));

        assert_eq!(
            effective_https_proxy(&settings),
            Some(ResolvedValue {
                env_name: "HTTP_PROXY",
                value: "http://upper-http:8080".to_string(),
            })
        );
    }

    #[test]
    fn https_proxy_prefers_https_specific_value() {
        let settings = ProxySettings {
            http_proxy: Some(ResolvedValue {
                env_name: "HTTP_PROXY",
                value: "http://http-proxy:8080".to_string(),
            }),
            https_proxy: Some(ResolvedValue {
                env_name: "HTTPS_PROXY",
                value: "http://https-proxy:8080".to_string(),
            }),
            all_proxy: None,
            no_proxy: None,
        };

        assert_eq!(
            effective_https_proxy(&settings),
            Some(ResolvedValue {
                env_name: "HTTPS_PROXY",
                value: "http://https-proxy:8080".to_string(),
            })
        );
    }
}
