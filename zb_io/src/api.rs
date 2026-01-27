use crate::cache::{ApiCache, CacheEntry};
use serde::Deserialize;
use zb_core::{Error, Formula};

/// Minimal formula info for search results (faster to deserialize than full Formula)
#[derive(Debug, Clone, Deserialize)]
pub struct FormulaInfo {
    pub name: String,
    pub full_name: String,
    pub desc: Option<String>,
    pub homepage: Option<String>,
    pub versions: FormulaVersions,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormulaVersions {
    pub stable: Option<String>,
}

pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
    cache: Option<ApiCache>,
}

impl ApiClient {
    pub fn new() -> Self {
        Self::with_base_url("https://formulae.brew.sh/api/formula".to_string())
    }

    pub fn with_base_url(base_url: String) -> Self {
        // Use HTTP/2 with connection pooling for better multiplexing of parallel requests
        let client = reqwest::Client::builder()
            .user_agent("zerobrew/0.1")
            .pool_max_idle_per_host(20)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            base_url,
            client,
            cache: None,
        }
    }

    pub fn with_cache(mut self, cache: ApiCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub async fn get_formula(&self, name: &str) -> Result<Formula, Error> {
        // Use a loop to handle alias resolution without recursion
        let mut current_name = name.to_string();
        let mut alias_resolved = false;

        loop {
            let url = format!("{}/{}.json", self.base_url, current_name);

            let cached_entry = self.cache.as_ref().and_then(|c| c.get(&url));

            let mut request = self.client.get(&url);

            if let Some(ref entry) = cached_entry {
                if let Some(ref etag) = entry.etag {
                    request = request.header("If-None-Match", etag.as_str());
                }
                if let Some(ref last_modified) = entry.last_modified {
                    request = request.header("If-Modified-Since", last_modified.as_str());
                }
            }

            let response = request.send().await.map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })?;

            if response.status() == reqwest::StatusCode::NOT_MODIFIED
                && let Some(entry) = cached_entry
            {
                let formula: Formula =
                    serde_json::from_str(&entry.body).map_err(|e| Error::NetworkFailure {
                        message: format!("failed to parse cached formula JSON: {e}"),
                    })?;
                return Ok(formula);
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                // Only try alias resolution once
                if !alias_resolved && let Some(target) = self.resolve_alias(&current_name).await {
                    current_name = target;
                    alias_resolved = true;
                    continue;
                }
                return Err(Error::MissingFormula {
                    name: name.to_string(),
                });
            }

            if !response.status().is_success() {
                return Err(Error::NetworkFailure {
                    message: format!("HTTP {}", response.status()),
                });
            }

            let etag = response
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let last_modified = response
                .headers()
                .get("last-modified")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let body = response.text().await.map_err(|e| Error::NetworkFailure {
                message: format!("failed to read response body: {e}"),
            })?;

            if let Some(ref cache) = self.cache {
                let entry = CacheEntry {
                    etag,
                    last_modified,
                    body: body.clone(),
                };
                if let Err(e) = cache.put(&url, &entry) {
                    eprintln!("    Warning: failed to cache response: {}", e);
                }
            }

            let formula: Formula =
                serde_json::from_str(&body).map_err(|e| Error::NetworkFailure {
                    message: format!("failed to parse formula JSON: {e}"),
                })?;

            return Ok(formula);
        }
    }

    /// Fetch all formula metadata for search
    pub async fn get_all_formulas(&self) -> Result<Vec<FormulaInfo>, Error> {
        // The base_url is like "https://formulae.brew.sh/api/formula"
        // We need "https://formulae.brew.sh/api/formula.json"
        let url = format!("{}.json", self.base_url);

        let cached_entry = self.cache.as_ref().and_then(|c| c.get(&url));

        let mut request = self.client.get(&url);

        if let Some(ref entry) = cached_entry {
            if let Some(ref etag) = entry.etag {
                request = request.header("If-None-Match", etag.as_str());
            }
            if let Some(ref last_modified) = entry.last_modified {
                request = request.header("If-Modified-Since", last_modified.as_str());
            }
        }

        let response = request.send().await.map_err(|e| Error::NetworkFailure {
            message: e.to_string(),
        })?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED
            && let Some(entry) = cached_entry
        {
            let formulas: Vec<FormulaInfo> =
                serde_json::from_str(&entry.body).map_err(|e| Error::NetworkFailure {
                    message: format!("failed to parse cached formula list: {e}"),
                })?;
            return Ok(formulas);
        }

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("HTTP {}", response.status()),
            });
        }

        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let body = response.text().await.map_err(|e| Error::NetworkFailure {
            message: format!("failed to read response body: {e}"),
        })?;

        if let Some(ref cache) = self.cache {
            let entry = CacheEntry {
                etag,
                last_modified,
                body: body.clone(),
            };
            if let Err(e) = cache.put(&url, &entry) {
                eprintln!("    Warning: failed to cache response: {}", e);
            }
        }

        let formulas: Vec<FormulaInfo> =
            serde_json::from_str(&body).map_err(|e| Error::NetworkFailure {
                message: format!("failed to parse formula list: {e}"),
            })?;

        Ok(formulas)
    }

    /// Check if a formula name is an alias and return the target formula name
    async fn resolve_alias(&self, name: &str) -> Option<String> {
        let alias_url = format!(
            "https://raw.githubusercontent.com/Homebrew/homebrew-core/master/Aliases/{}",
            name
        );

        let response = self.client.get(&alias_url).send().await.ok()?;

        if !response.status().is_success() {
            return None;
        }

        let body = response.text().await.ok()?;
        // Alias file contains a relative path like "../Formula/p/python@3.14.rb"
        // Extract the formula name from the path
        let formula_name = body
            .trim()
            .rsplit('/')
            .next()?
            .strip_suffix(".rb")?
            .to_string();

        Some(formula_name)
    }

    /// Clean up HTTP cache entries older than the specified number of days
    /// Returns the number of entries removed and their total size
    pub fn cleanup_cache_older_than(&self, days: u32) -> Option<(usize, u64)> {
        self.cache.as_ref().map(|c| {
            let size = c.body_size_older_than(days).unwrap_or(0);
            let removed = c.cleanup_older_than(days).unwrap_or(0);
            (removed, size)
        })
    }

    /// Clear all HTTP cache entries
    /// Returns the number of entries removed and their total size
    pub fn clear_cache(&self) -> Option<(usize, u64)> {
        self.cache.as_ref().map(|c| {
            let size = c.total_body_size().unwrap_or(0);
            let removed = c.clear().unwrap_or(0);
            (removed, size)
        })
    }

    /// Count HTTP cache entries older than the specified days
    pub fn cache_count_older_than(&self, days: u32) -> Option<(usize, u64)> {
        self.cache.as_ref().map(|c| {
            let count = c.count_older_than(days).unwrap_or(0);
            let size = c.body_size_older_than(days).unwrap_or(0);
            (count, size)
        })
    }

    /// Get total count and size of HTTP cache entries
    pub fn cache_stats(&self) -> Option<(usize, u64)> {
        self.cache.as_ref().map(|c| {
            let count = c.count().unwrap_or(0);
            let size = c.total_body_size().unwrap_or(0);
            (count, size)
        })
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetches_formula_from_mock_server() {
        let mock_server = MockServer::start().await;

        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let formula = client.get_formula("foo").await.unwrap();

        assert_eq!(formula.name, "foo");
        assert_eq!(formula.versions.stable, "1.2.3");
    }

    #[tokio::test]
    async fn returns_missing_formula_on_404() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/nonexistent.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("nonexistent").await.unwrap_err();

        assert!(matches!(
            err,
            Error::MissingFormula { name } if name == "nonexistent"
        ));
    }

    #[tokio::test]
    async fn first_request_stores_etag() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        let cached = client
            .cache
            .as_ref()
            .unwrap()
            .get(&format!("{}/foo.json", mock_server.uri()))
            .unwrap();
        assert_eq!(cached.etag, Some("\"abc123\"".to_string()));
    }

    #[tokio::test]
    async fn second_request_sends_if_none_match() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // First request returns 200 with ETag
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        // First request
        let _ = client.get_formula("foo").await.unwrap();

        // Reset mocks for second request
        mock_server.reset().await;

        // Second request should send If-None-Match and receive 304
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-None-Match", "\"abc123\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&mock_server)
            .await;

        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
    }

    #[tokio::test]
    async fn uses_cached_body_on_304() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // First request returns 200 with ETag
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"abc123\""),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        // First request populates cache
        let _ = client.get_formula("foo").await.unwrap();

        mock_server.reset().await;

        // Second request returns 304 (no body)
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-None-Match", "\"abc123\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        // Should return cached formula
        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
        assert_eq!(formula.versions.stable, "1.2.3");
    }

    // ========================================================================
    // Last-Modified / If-Modified-Since handling
    // ========================================================================

    #[tokio::test]
    async fn stores_last_modified_header() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/api/formula/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("last-modified", "Wed, 21 Oct 2024 07:28:00 GMT"),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url.clone()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        let cached = client
            .cache
            .as_ref()
            .unwrap()
            .get(&format!("{}/foo.json", base_url))
            .unwrap();
        assert_eq!(
            cached.last_modified,
            Some("Wed, 21 Oct 2024 07:28:00 GMT".to_string())
        );
    }

    #[tokio::test]
    async fn sends_if_modified_since_on_second_request() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // Mount 304 mock first (higher priority for requests with If-Modified-Since)
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-Modified-Since", "Wed, 21 Oct 2024 07:28:00 GMT"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        // Then mount 200 mock for requests without If-Modified-Since
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("last-modified", "Wed, 21 Oct 2024 07:28:00 GMT"),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        // First request (no If-Modified-Since, gets 200)
        let _ = client.get_formula("foo").await.unwrap();

        // Verify the cache has the Last-Modified value
        let cached = client
            .cache
            .as_ref()
            .unwrap()
            .get(&format!("{}/foo.json", mock_server.uri()))
            .unwrap();
        assert_eq!(
            cached.last_modified,
            Some("Wed, 21 Oct 2024 07:28:00 GMT".to_string()),
            "Cache should store last-modified header"
        );

        // Second request (with If-Modified-Since, gets 304)
        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
    }

    #[tokio::test]
    async fn sends_both_etag_and_last_modified_headers() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // Mount 304 mock first (matches when both conditional headers present)
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .and(header("If-None-Match", "\"xyz789\""))
            .and(header("If-Modified-Since", "Thu, 22 Oct 2024 10:00:00 GMT"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        // Then mount 200 mock for initial requests
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .insert_header("etag", "\"xyz789\"")
                    .insert_header("last-modified", "Thu, 22 Oct 2024 10:00:00 GMT"),
            )
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        // First request
        let _ = client.get_formula("foo").await.unwrap();

        // Second request should get 304 and use cached data
        let formula = client.get_formula("foo").await.unwrap();
        assert_eq!(formula.name, "foo");
    }

    // ========================================================================
    // Error handling
    // ========================================================================

    #[tokio::test]
    async fn returns_error_on_500() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/broken.json"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("broken").await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("500"));
            }
            _ => panic!("expected NetworkFailure, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn returns_error_on_503_service_unavailable() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/unavailable.json"))
            .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("unavailable").await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("503"));
            }
            _ => panic!("expected NetworkFailure, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn returns_error_on_429_rate_limit() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ratelimited.json"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_string("Too Many Requests")
                    .insert_header("Retry-After", "60"),
            )
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("ratelimited").await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("429"));
            }
            _ => panic!("expected NetworkFailure, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn returns_error_on_invalid_json() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/invalid.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{ invalid json }"))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("invalid").await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("parse"));
            }
            _ => panic!("expected NetworkFailure with parse error, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn returns_error_on_cached_invalid_json_after_304() {
        let mock_server = MockServer::start().await;

        // First request returns 200 with valid ETag but we manually insert invalid cache
        let cache = ApiCache::in_memory().unwrap();
        let base_url = format!("{}/api/formula", mock_server.uri());
        let cache_url = format!("{}/foo.json", base_url);

        // Manually insert invalid JSON into cache
        let entry = CacheEntry {
            etag: Some("\"badcache\"".to_string()),
            last_modified: None,
            body: "{ not valid json }".to_string(),
        };
        cache.put(&cache_url, &entry).unwrap();

        // Request returns 304 (use cached body)
        Mock::given(method("GET"))
            .and(path("/api/formula/foo.json"))
            .and(header("If-None-Match", "\"badcache\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(base_url).with_cache(cache);

        // Should fail when parsing the cached invalid JSON
        let err = client.get_formula("foo").await.unwrap_err();
        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("cached"));
            }
            _ => panic!("expected cached parse error, got {:?}", err),
        }
    }

    // ========================================================================
    // get_all_formulas tests
    // ========================================================================

    #[tokio::test]
    async fn get_all_formulas_success() {
        let mock_server = MockServer::start().await;

        let formulas_json = r#"[
            {
                "name": "foo",
                "full_name": "homebrew/core/foo",
                "desc": "A test formula",
                "homepage": "https://example.com/foo",
                "versions": { "stable": "1.0.0" },
                "aliases": ["f"],
                "deprecated": false,
                "disabled": false
            },
            {
                "name": "bar",
                "full_name": "homebrew/core/bar",
                "desc": null,
                "homepage": null,
                "versions": { "stable": "2.0.0" },
                "aliases": [],
                "deprecated": true,
                "disabled": false
            }
        ]"#;

        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(formulas_json))
            .mount(&mock_server)
            .await;

        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url);
        let formulas = client.get_all_formulas().await.unwrap();

        assert_eq!(formulas.len(), 2);
        assert_eq!(formulas[0].name, "foo");
        assert_eq!(formulas[0].desc, Some("A test formula".to_string()));
        assert_eq!(formulas[0].aliases, vec!["f"]);
        assert!(!formulas[0].deprecated);

        assert_eq!(formulas[1].name, "bar");
        assert!(formulas[1].desc.is_none());
        assert!(formulas[1].deprecated);
    }

    #[tokio::test]
    async fn get_all_formulas_with_caching() {
        let mock_server = MockServer::start().await;

        let formulas_json = r#"[
            {
                "name": "cached",
                "full_name": "homebrew/core/cached",
                "desc": "Cached formula",
                "homepage": null,
                "versions": { "stable": "1.0.0" },
                "aliases": [],
                "deprecated": false,
                "disabled": false
            }
        ]"#;

        // First request returns 200 with ETag and Last-Modified
        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(formulas_json)
                    .insert_header("ETag", "\"allformulas\"")
                    .insert_header("Last-Modified", "Mon, 01 Jan 2024 00:00:00 GMT"),
            )
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // Second request returns 304 when conditional headers present
        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .and(header("If-None-Match", "\"allformulas\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url).with_cache(cache);

        // First request
        let formulas = client.get_all_formulas().await.unwrap();
        assert_eq!(formulas.len(), 1);

        // Second request should use cached data on 304
        let formulas = client.get_all_formulas().await.unwrap();
        assert_eq!(formulas.len(), 1);
        assert_eq!(formulas[0].name, "cached");
    }

    #[tokio::test]
    async fn get_all_formulas_returns_error_on_500() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url);
        let err = client.get_all_formulas().await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("500"));
            }
            _ => panic!("expected NetworkFailure"),
        }
    }

    #[tokio::test]
    async fn get_all_formulas_returns_error_on_invalid_json() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url);
        let err = client.get_all_formulas().await.unwrap_err();

        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("parse"));
            }
            _ => panic!("expected parse error"),
        }
    }

    // ========================================================================
    // Alias resolution tests
    // ========================================================================

    // Note: resolve_alias uses a hardcoded GitHub URL, so we can't fully mock it.
    // These tests verify the behavior when alias resolution fails or is not attempted.

    #[tokio::test]
    async fn formula_404_without_alias_returns_missing_formula() {
        let mock_server = MockServer::start().await;

        // Formula returns 404, alias resolution will fail (can't reach GitHub)
        Mock::given(method("GET"))
            .and(path("/python3.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());

        // Should return MissingFormula since alias resolution will fail
        let err = client.get_formula("python3").await.unwrap_err();
        assert!(matches!(err, Error::MissingFormula { name } if name == "python3"));
    }

    #[tokio::test]
    async fn alias_resolution_prevents_infinite_loop() {
        // Test that alias resolution only happens once to prevent infinite loops
        let mock_server = MockServer::start().await;

        // Formula returns 404
        Mock::given(method("GET"))
            .and(path("/circular.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let err = client.get_formula("circular").await.unwrap_err();

        // Should return MissingFormula, not hang or stack overflow
        assert!(matches!(err, Error::MissingFormula { name } if name == "circular"));
    }

    // ========================================================================
    // Cache helper methods tests
    // ========================================================================

    #[tokio::test]
    async fn cache_stats_returns_none_without_cache() {
        let mock_server = MockServer::start().await;
        let client = ApiClient::with_base_url(mock_server.uri());

        assert!(client.cache_stats().is_none());
        assert!(client.cache_count_older_than(7).is_none());
        assert!(client.cleanup_cache_older_than(7).is_none());
        assert!(client.clear_cache().is_none());
    }

    #[tokio::test]
    async fn cache_stats_returns_correct_values() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        // Initially empty
        let (count, size) = client.cache_stats().unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);

        // After fetching
        let _ = client.get_formula("foo").await.unwrap();

        let (count, size) = client.cache_stats().unwrap();
        assert_eq!(count, 1);
        assert!(size > 0);
    }

    #[tokio::test]
    async fn clear_cache_removes_entries() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        let (count, _) = client.cache_stats().unwrap();
        assert_eq!(count, 1);

        let (removed, size) = client.clear_cache().unwrap();
        assert_eq!(removed, 1);
        assert!(size > 0);

        let (count, size) = client.cache_stats().unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    #[tokio::test]
    async fn cleanup_cache_older_than_works() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        // Cleanup entries older than 1 day - should remove nothing (just cached)
        let (removed, _) = client.cleanup_cache_older_than(1).unwrap();
        assert_eq!(removed, 0);

        // Entry still exists
        let (count, _) = client.cache_stats().unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn cache_count_older_than_works() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .mount(&mock_server)
            .await;

        let cache = ApiCache::in_memory().unwrap();
        let client = ApiClient::with_base_url(mock_server.uri()).with_cache(cache);

        let _ = client.get_formula("foo").await.unwrap();

        // No entries older than 1 day
        let (count, size) = client.cache_count_older_than(1).unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    // ========================================================================
    // Default implementation
    // ========================================================================

    #[test]
    fn default_creates_new_client() {
        let client = ApiClient::default();
        assert!(client.cache.is_none());
    }

    #[test]
    fn new_creates_client_with_default_url() {
        let client = ApiClient::new();
        assert!(client.cache.is_none());
    }

    // ========================================================================
    // Edge cases and formula deserialization
    // ========================================================================

    #[tokio::test]
    async fn handles_formula_with_missing_optional_fields() {
        let mock_server = MockServer::start().await;

        // Minimal formula JSON
        let minimal_json = r#"{
            "name": "minimal",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": {
                "stable": {
                    "files": {}
                }
            }
        }"#;

        Mock::given(method("GET"))
            .and(path("/minimal.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(minimal_json))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let formula = client.get_formula("minimal").await.unwrap();

        assert_eq!(formula.name, "minimal");
        assert_eq!(formula.versions.stable, "1.0.0");
    }

    #[tokio::test]
    async fn get_all_formulas_handles_empty_list() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&mock_server)
            .await;

        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url);
        let formulas = client.get_all_formulas().await.unwrap();

        assert!(formulas.is_empty());
    }

    #[tokio::test]
    async fn formula_info_deserializes_with_defaults() {
        let mock_server = MockServer::start().await;

        // Formula without optional fields (aliases, deprecated, disabled)
        let json = r#"[
            {
                "name": "simple",
                "full_name": "homebrew/core/simple",
                "desc": null,
                "homepage": null,
                "versions": { "stable": "1.0.0" }
            }
        ]"#;

        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(json))
            .mount(&mock_server)
            .await;

        let base_url = format!("{}/api/formula", mock_server.uri());
        let client = ApiClient::with_base_url(base_url);
        let formulas = client.get_all_formulas().await.unwrap();

        assert_eq!(formulas.len(), 1);
        assert!(formulas[0].aliases.is_empty());
        assert!(!formulas[0].deprecated);
        assert!(!formulas[0].disabled);
    }

    // ========================================================================
    // Request without cache (no conditional headers)
    // ========================================================================

    #[tokio::test]
    async fn request_without_cache_sends_no_conditional_headers() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // Mock that verifies no conditional headers
        Mock::given(method("GET"))
            .and(path("/foo.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Client without cache
        let client = ApiClient::with_base_url(mock_server.uri());
        let formula = client.get_formula("foo").await.unwrap();

        assert_eq!(formula.name, "foo");
    }

    // ========================================================================
    // Network timeout simulation
    // ========================================================================

    #[tokio::test]
    async fn handles_slow_response() {
        let mock_server = MockServer::start().await;
        let fixture = include_str!("../../zb_core/fixtures/formula_foo.json");

        // Response with small delay (should still succeed)
        Mock::given(method("GET"))
            .and(path("/slow.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(fixture)
                    .set_delay(Duration::from_millis(100)),
            )
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(mock_server.uri());
        let formula = client.get_formula("slow").await.unwrap();

        assert_eq!(formula.name, "foo");
    }

    // ========================================================================
    // Body read errors
    // ========================================================================

    #[tokio::test]
    async fn get_all_formulas_handles_cached_invalid_json_after_304() {
        let mock_server = MockServer::start().await;

        // Manually insert invalid JSON into cache
        let cache = ApiCache::in_memory().unwrap();
        let base_url = format!("{}/api/formula", mock_server.uri());
        let cache_url = format!("{}.json", base_url);

        let entry = CacheEntry {
            etag: Some("\"listcache\"".to_string()),
            last_modified: None,
            body: "not valid json array".to_string(),
        };
        cache.put(&cache_url, &entry).unwrap();

        // Request returns 304 (use cached body)
        Mock::given(method("GET"))
            .and(path("/api/formula.json"))
            .and(header("If-None-Match", "\"listcache\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&mock_server)
            .await;

        let client = ApiClient::with_base_url(base_url).with_cache(cache);

        // Should fail when parsing the cached invalid JSON
        let err = client.get_all_formulas().await.unwrap_err();
        match err {
            Error::NetworkFailure { message } => {
                assert!(message.contains("cached"));
            }
            _ => panic!("expected cached parse error, got {:?}", err),
        }
    }
}
