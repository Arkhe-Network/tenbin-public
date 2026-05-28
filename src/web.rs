use log::error;
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};
use std::collections::HashMap;

pub struct GoogleGroundingLayer {
    pub api_key: Option<String>,
    pub cx: Option<String>,
    pub serpapi_key: Option<String>,
    pub default_engine: String,
    pub session_queries: u32,
    pub total_results_fetched: u32,
}

impl GoogleGroundingLayer {
    pub fn new(
        api_key: Option<String>,
        cx: Option<String>,
        serpapi_key: Option<String>,
        default_engine: &str,
    ) -> Self {
        let valid_engines = vec!["google", "google_news", "google_scholar", "google_images"];
        let engine = if valid_engines.contains(&default_engine) {
            default_engine.to_string()
        } else {
            "google".to_string()
        };

        Self {
            api_key: api_key.or_else(|| std::env::var("GOOGLE_API_KEY").ok()),
            cx: cx.or_else(|| std::env::var("GOOGLE_CX").ok()),
            serpapi_key: serpapi_key.or_else(|| std::env::var("SERPAPI_KEY").ok()),
            default_engine: engine,
            session_queries: 0,
            total_results_fetched: 0,
        }
    }

    fn mock_search(&self, query: &str, engine: &str, num_results: usize) -> Value {
        let mut hasher = Sha3_256::new();
        hasher.update(query.as_bytes());
        let seed = hex::encode(hasher.finalize());
        let seed_bytes: [u8; 32] = {
            let mut arr = [0u8; 32];
            let bytes = hex::decode(&seed).unwrap();
            arr.copy_from_slice(&bytes[..32]);
            arr
        };

        let mut rng = StdRng::from_seed(seed_bytes);
        let domains = vec![
            "arxiv.org", "nature.com", "techcrunch.com", "github.com", "wikipedia.org", "medium.com", "reuters.com"
        ];

        let mut results = Vec::new();
        for i in 0..num_results {
            let domain = domains[rng.gen_range(0..domains.len())];
            let snippet_seed = &seed[..8];
            results.push(json!({
                "title": format!("[{}] Result {} for '{}...'", engine.to_uppercase(), i + 1, &query[..std::cmp::min(query.len(), 40)]),
                "link": format!("https://{}/article/{}-{}", domain, snippet_seed, i),
                "displayLink": domain,
                "snippet": "Mock snippet for demonstration.",
                "htmlSnippet": "<b>Mock</b> snippet...",
                "htmlTitle": format!("Result {} — {}", i + 1, &query[..std::cmp::min(query.len(), 30)]),
                "formattedUrl": format!("{}/article/{}-{}", domain, snippet_seed, i),
                "pagemap": {
                    "metatags": [{"og:type": "article", "og:title": &query[..std::cmp::min(query.len(), 50)]}]
                }
            }));
        }

        let search_time = 0.1 + rng.gen::<f64>() * 0.4;
        let total_results_val = rng.gen_range(10000..10000000);

        json!({
            "query": query,
            "engine": engine,
            "results": results,
            "total_results": num_results,
            "searchInformation": {
                "searchTime": (search_time * 1000.0).round() / 1000.0,
                "formattedSearchTime": format!("{:.3}s", search_time),
                "totalResults": total_results_val.to_string(),
                "formattedTotalResults": format!("{}", total_results_val) // could format with commas but standard format! is fine
            },
            "mock": true
        })
    }

    pub fn search(
        &mut self,
        query: &str,
        engine: Option<&str>,
        num_results: usize,
    ) -> Value {
        let eng = engine.unwrap_or(&self.default_engine);

        // the real HTTP requests aren't fully implemented in this port to avoid async reqwest
        // dependencies or network calls for a mock app. We log a warning and fall back to mock.
        log::warn!("⚠️  No Google API keys or real requests implemented — using mock search");

        let result = self.mock_search(query, eng, num_results);
        self.session_queries += 1;
        self.total_results_fetched += result["results"].as_array().unwrap_or(&vec![]).len() as u32;
        result
    }

    pub fn synthesize_context(&self, search_results: &Value, max_snippets: usize) -> String {
        if !search_results.get("results").is_some() || search_results["results"].as_array().unwrap().is_empty() {
            return "".to_string();
        }

        let engine = search_results["engine"].as_str().unwrap_or("google").to_uppercase();
        let query = search_results["query"].as_str().unwrap_or("");

        let mut lines = Vec::new();
        lines.push(format!("[WEB-GROUNDED CONTEXT | {}]", engine));
        lines.push(format!("Query: {}", query));

        if let Some(info) = search_results.get("searchInformation") {
            let total = info["formattedTotalResults"].as_str().unwrap_or("N/A");
            let time = info["formattedSearchTime"].as_str().unwrap_or("N/A");
            lines.push(format!("Results: {} in {}", total, time));
        }

        lines.push("-".repeat(50));

        let results = search_results["results"].as_array().unwrap();
        for (i, r) in results.iter().take(max_snippets).enumerate() {
            let title = r["title"].as_str().unwrap_or("Untitled");
            let link = r["link"].as_str().unwrap_or("");
            let domain = r["displayLink"].as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                if let Ok(url) = url::Url::parse(link) {
                    url.host_str().unwrap_or("").to_string()
                } else {
                    "".to_string()
                }
            });

            lines.push(format!("[{}] {}", i + 1, title));
            lines.push(format!("    Source: {}", domain));

            let snippet = r["snippet"].as_str().unwrap_or("");
            if !snippet.is_empty() {
                let trunc = if snippet.len() > 250 { format!("{}...", &snippet[..250]) } else { snippet.to_string() };
                lines.push(format!("    → {}", trunc));
            }
            lines.push("".to_string());
        }

        lines.join("\n")
    }

    pub fn to_peptide_descriptor(&self, search_results: &Value) -> HashMap<String, Value> {
        let query = search_results["query"].as_str().unwrap_or("");
        let query_trunc = &query[..std::cmp::min(query.len(), 20)];

        let search_json = serde_json::to_string(&search_results).unwrap_or_default();
        let mut hasher = Sha3_256::new();
        hasher.update(search_json.as_bytes());
        let hash = hex::encode(hasher.finalize())[..16].to_string();

        let engine = search_results["engine"].clone();
        let results_count = search_results["total_results"].clone();
        let search_time = search_results.get("searchInformation")
            .and_then(|info| info.get("searchTime"))
            .cloned()
            .unwrap_or(json!(0.0));

        let mut desc = HashMap::new();
        desc.insert("sequence".to_string(), json!(format!("google:{}", query_trunc)));
        desc.insert("source_code_hash".to_string(), json!(hash));

        let mut endpoints = HashMap::new();
        endpoints.insert("engine".to_string(), engine);
        endpoints.insert("results_count".to_string(), results_count);
        endpoints.insert("search_time".to_string(), search_time);
        desc.insert("api_endpoints".to_string(), json!(endpoints));

        desc.insert("subscription_model".to_string(), json!("GOOGLE-per-query"));
        desc.insert("zero_trust".to_string(), json!(true));
        desc.insert("results_fetched".to_string(), json!(self.total_results_fetched));

        desc
    }
}
