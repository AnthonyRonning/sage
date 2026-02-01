//! Brave Search API client (Pro)
//!
//! Full-featured client with:
//! - AI Summarizer integration
//! - Rich data callbacks (weather, stocks, sports, etc.)
//! - Location-aware search
//! - Freshness filtering
//! - FAQ and discussion results

use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

const BRAVE_API_BASE: &str = "https://api.search.brave.com/res/v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum BraveError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },
}

/// Search options for customizing queries
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Number of results (max 20)
    pub count: Option<u32>,
    /// Freshness filter: "pd" (24h), "pw" (week), "pm" (month), "py" (year)
    pub freshness: Option<String>,
    /// Location for local results (city name or "city, state")
    pub location: Option<String>,
    /// User's timezone (IANA format)
    pub timezone: Option<String>,
}

#[derive(Clone)]
pub struct BraveClient {
    client: reqwest::Client,
    api_key: Arc<String>,
}

impl BraveClient {
    pub fn new(api_key: String) -> Result<Self, BraveError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("Sage/0.1.0")
            .build()?;

        Ok(Self {
            client,
            api_key: Arc::new(api_key),
        })
    }

    /// Perform a search with full Pro features
    pub async fn search(
        &self,
        query: &str,
        options: Option<SearchOptions>,
    ) -> Result<SearchResponse, BraveError> {
        let opts = options.unwrap_or_default();
        let url = format!("{}/web/search", BRAVE_API_BASE);

        // Build query parameters
        let mut params = vec![
            ("q", query.to_string()),
            ("summary", "1".to_string()), // Enable AI summarizer
            ("extra_snippets", "true".to_string()), // Get additional context
            ("enable_rich_callback", "1".to_string()), // Enable rich data (Pro)
            ("spellcheck", "true".to_string()), // Auto-correct typos
        ];

        if let Some(c) = opts.count {
            params.push(("count", c.min(20).to_string()));
        }

        if let Some(ref freshness) = opts.freshness {
            params.push(("freshness", freshness.clone()));
        }

        // Build request with location headers if provided
        let mut request = self
            .client
            .get(&url)
            .header("X-Subscription-Token", self.api_key.as_str())
            .header("Accept", "application/json");

        // Add location headers for local results
        if let Some(ref tz) = opts.timezone {
            request = request.header("x-loc-timezone", tz.as_str());
        }

        if let Some(ref location) = opts.location {
            // Parse "city, state" format
            let parts: Vec<&str> = location.split(',').map(|s| s.trim()).collect();
            if !parts.is_empty() {
                request = request.header("x-loc-city", parts[0]);
            }
            if parts.len() > 1 {
                request = request.header("x-loc-state-name", parts[1]);
            }
        }

        let response = request.query(&params).send().await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(BraveError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let mut search_response: SearchResponse = response.json().await?;

        // Automatically fetch AI summary if available
        if let Some(ref summarizer) = search_response.summarizer {
            debug!("Fetching Brave AI summary...");
            match self.fetch_summary(&summarizer.key).await {
                Ok(summary_response) => {
                    search_response.summary_text = summary_response.extract_text();
                }
                Err(e) => {
                    warn!("Failed to fetch Brave summary: {}", e);
                }
            }
        }

        // Automatically fetch rich data if available (weather, stocks, etc.)
        if let Some(ref rich) = search_response.rich {
            info!("Rich data available: {:?}", rich.hint.vertical);
            match self.fetch_rich(&rich.hint.callback_key).await {
                Ok(rich_response) => {
                    search_response.rich_data = Some(rich_response);
                }
                Err(e) => {
                    warn!("Failed to fetch rich data: {}", e);
                }
            }
        }

        Ok(search_response)
    }

    /// Fetch AI summary using the summarizer key
    async fn fetch_summary(&self, key: &str) -> Result<SummarizerResponse, BraveError> {
        let url = format!("{}/summarizer/search", BRAVE_API_BASE);

        let response = self
            .client
            .get(&url)
            .header("X-Subscription-Token", self.api_key.as_str())
            .header("Accept", "application/json")
            .query(&[("key", key)])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(BraveError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        Ok(response.json().await?)
    }

    /// Fetch rich data (weather, stocks, sports, etc.) using callback key
    async fn fetch_rich(&self, callback_key: &str) -> Result<RichResponse, BraveError> {
        let url = format!("{}/web/rich", BRAVE_API_BASE);

        let response = self
            .client
            .get(&url)
            .header("X-Subscription-Token", self.api_key.as_str())
            .header("Accept", "application/json")
            .query(&[("callback_key", callback_key)])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(BraveError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        Ok(response.json().await?)
    }
}

impl std::fmt::Debug for BraveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BraveClient")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub query: Option<QueryInfo>,
    pub web: Option<WebResults>,
    pub news: Option<NewsResults>,
    pub faq: Option<FaqResults>,
    pub discussions: Option<DiscussionResults>,
    pub infobox: Option<Infobox>,
    pub summarizer: Option<Summarizer>,
    pub rich: Option<RichHint>,
    /// Populated after fetching summary
    #[serde(skip)]
    pub summary_text: Option<String>,
    /// Populated after fetching rich callback
    #[serde(skip)]
    pub rich_data: Option<RichResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryInfo {
    pub original: Option<String>,
    pub altered: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebResults {
    pub results: Option<Vec<SearchResult>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsResults {
    pub results: Option<Vec<NewsResult>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FaqResults {
    pub results: Option<Vec<FaqResult>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionResults {
    pub results: Option<Vec<DiscussionResult>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub age: Option<String>,
    pub extra_snippets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsResult {
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub age: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FaqResult {
    pub question: String,
    pub answer: String,
    pub title: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionResult {
    pub title: String,
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Infobox {
    pub title: Option<String>,
    pub description: Option<String>,
    pub long_desc: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Summarizer {
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RichHint {
    #[serde(rename = "type")]
    pub rich_type: Option<String>,
    pub hint: RichHintDetails,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RichHintDetails {
    pub vertical: String,
    pub callback_key: String,
}

// ============================================================================
// Summarizer Response
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct SummarizerResponse {
    pub status: Option<String>,
    pub summary: Option<Vec<SummaryItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SummaryItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub data: Option<serde_json::Value>,
}

impl SummarizerResponse {
    pub fn extract_text(&self) -> Option<String> {
        let items = self.summary.as_ref()?;
        let mut text = String::new();

        for item in items {
            if item.item_type == "token" {
                if let Some(data) = &item.data {
                    if let Some(s) = data.as_str() {
                        text.push_str(s);
                    }
                }
            }
        }

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

// ============================================================================
// Rich Data Response (Weather, Stocks, Sports, etc.)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct RichResponse {
    #[serde(rename = "type")]
    pub response_type: Option<String>,
    pub results: Option<Vec<RichResult>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RichResult {
    #[serde(rename = "type")]
    pub result_type: Option<String>,
    pub subtype: Option<String>,
    /// Raw data - structure varies by subtype (weather, stock, etc.)
    #[serde(flatten)]
    pub data: serde_json::Value,
}

impl RichResponse {
    /// Format rich data for display
    pub fn format(&self) -> Option<String> {
        let results = self.results.as_ref()?;
        let first = results.first()?;
        first.format()
    }
}

impl RichResult {
    /// Format a single rich result for display
    pub fn format(&self) -> Option<String> {
        let subtype = self.subtype.as_deref()?;

        match subtype {
            "weather" => self.format_weather(),
            "stock" => self.format_stock(),
            "currency" => self.format_currency(),
            "cryptocurrency" => self.format_crypto(),
            "calculator" => self.format_calculator(),
            "unit_conversion" => self.format_unit_conversion(),
            "definitions" => self.format_definition(),
            _ => {
                // For unknown types, return raw JSON summary
                Some(format!(
                    "**{}**: {}",
                    subtype,
                    serde_json::to_string_pretty(&self.data).unwrap_or_default()
                ))
            }
        }
    }

    fn format_weather(&self) -> Option<String> {
        let mut output = String::new();

        // Extract weather data from the API response structure
        if let Some(weather) = self.data.get("weather") {
            // Location info
            if let Some(location) = weather.get("location") {
                let name = location
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Unknown");
                let state = location.get("state").and_then(|s| s.as_str()).unwrap_or("");
                output.push_str(&format!("**Weather for {}, {}**\n\n", name, state));
            }

            // Current conditions (field is "current_weather", temps in Celsius)
            if let Some(current) = weather.get("current_weather") {
                output.push_str("**Current Conditions:**\n");
                if let Some(temp_c) = current.get("temp").and_then(|t| t.as_f64()) {
                    let temp_f = temp_c * 9.0 / 5.0 + 32.0;
                    output.push_str(&format!("  Temperature: {:.0}°F\n", temp_f));
                }
                if let Some(feels_c) = current.get("feels_like").and_then(|t| t.as_f64()) {
                    let feels_f = feels_c * 9.0 / 5.0 + 32.0;
                    output.push_str(&format!("  Feels like: {:.0}°F\n", feels_f));
                }
                // Description is nested: weather.description
                if let Some(desc) = current
                    .get("weather")
                    .and_then(|w| w.get("description"))
                    .and_then(|d| d.as_str())
                {
                    output.push_str(&format!("  Conditions: {}\n", desc));
                }
                if let Some(humidity) = current.get("humidity") {
                    output.push_str(&format!("  Humidity: {}%\n", humidity));
                }
                // Wind is nested: wind.speed (m/s, convert to mph)
                if let Some(wind_ms) = current
                    .get("wind")
                    .and_then(|w| w.get("speed"))
                    .and_then(|s| s.as_f64())
                {
                    let wind_mph = wind_ms * 2.237;
                    output.push_str(&format!("  Wind: {:.0} mph\n", wind_mph));
                }
                output.push('\n');
            }

            // Weather alerts (important!)
            if let Some(alerts) = weather.get("alerts").and_then(|a| a.as_array()) {
                if !alerts.is_empty() {
                    output.push_str("**⚠️ Weather Alerts:**\n");
                    for alert in alerts.iter().take(3) {
                        if let Some(event) = alert.get("event").and_then(|e| e.as_str()) {
                            output.push_str(&format!("  • {}\n", event));
                            if let Some(desc) = alert.get("description").and_then(|d| d.as_str()) {
                                // Truncate long descriptions
                                let short_desc: String = desc.chars().take(200).collect();
                                output.push_str(&format!(
                                    "    {}{}\n",
                                    short_desc,
                                    if desc.len() > 200 { "..." } else { "" }
                                ));
                            }
                        }
                    }
                    output.push('\n');
                }
            }

            // Daily forecast
            if let Some(daily) = weather.get("daily").and_then(|d| d.as_array()) {
                output.push_str("**Forecast:**\n");
                for (i, day) in daily.iter().take(5).enumerate() {
                    // Get date or fallback to day number
                    let day_name = day
                        .get("date_i18n")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| match i {
                            0 => "Today".to_string(),
                            1 => "Tomorrow".to_string(),
                            _ => format!("Day {}", i + 1),
                        });

                    // Temperature is nested: temperature.max / temperature.min
                    let high = day
                        .get("temperature")
                        .and_then(|t| t.get("max"))
                        .and_then(|v| v.as_f64())
                        .map(|c| format!("{:.0}°F", c * 9.0 / 5.0 + 32.0)) // Convert C to F
                        .unwrap_or_default();
                    let low = day
                        .get("temperature")
                        .and_then(|t| t.get("min"))
                        .and_then(|v| v.as_f64())
                        .map(|c| format!("{:.0}°F", c * 9.0 / 5.0 + 32.0))
                        .unwrap_or_default();

                    // Description is nested: weather.description
                    let desc = day
                        .get("weather")
                        .and_then(|w| w.get("description"))
                        .and_then(|d| d.as_str())
                        .unwrap_or("");

                    output.push_str(&format!(
                        "  {} - High: {}, Low: {} - {}\n",
                        day_name, high, low, desc
                    ));
                }
            }
        } else {
            output.push_str("**Weather data:**\n");
            output.push_str(&serde_json::to_string_pretty(&self.data).unwrap_or_default());
        }

        Some(output)
    }

    fn format_stock(&self) -> Option<String> {
        let mut output = String::from("**Stock:**\n\n");

        if let Some(symbol) = self.data.get("symbol").and_then(|s| s.as_str()) {
            output.push_str(&format!("Symbol: {}\n", symbol));
        }
        if let Some(name) = self.data.get("name").and_then(|s| s.as_str()) {
            output.push_str(&format!("Name: {}\n", name));
        }
        if let Some(price) = self.data.get("price") {
            output.push_str(&format!("Price: ${}\n", price));
        }
        if let Some(change) = self.data.get("change") {
            output.push_str(&format!("Change: {}\n", change));
        }
        if let Some(change_pct) = self.data.get("change_percent") {
            output.push_str(&format!("Change %: {}%\n", change_pct));
        }

        Some(output)
    }

    fn format_currency(&self) -> Option<String> {
        let mut output = String::from("**Currency Conversion:**\n\n");
        output.push_str(&serde_json::to_string_pretty(&self.data).unwrap_or_default());
        Some(output)
    }

    fn format_crypto(&self) -> Option<String> {
        let mut output = String::from("**Cryptocurrency:**\n\n");

        if let Some(name) = self.data.get("name").and_then(|s| s.as_str()) {
            output.push_str(&format!("Name: {}\n", name));
        }
        if let Some(symbol) = self.data.get("symbol").and_then(|s| s.as_str()) {
            output.push_str(&format!("Symbol: {}\n", symbol));
        }
        if let Some(price) = self.data.get("price") {
            output.push_str(&format!("Price: ${}\n", price));
        }
        if let Some(change) = self.data.get("change_24h") {
            output.push_str(&format!("24h Change: {}%\n", change));
        }

        Some(output)
    }

    fn format_calculator(&self) -> Option<String> {
        self.data
            .get("result")
            .map(|result| format!("**Calculator:** {}", result))
    }

    fn format_unit_conversion(&self) -> Option<String> {
        let mut output = String::from("**Unit Conversion:**\n");
        if let Some(result) = self.data.get("result").and_then(|r| r.as_str()) {
            output.push_str(result);
        }
        Some(output)
    }

    fn format_definition(&self) -> Option<String> {
        let mut output = String::from("**Definition:**\n\n");

        if let Some(word) = self.data.get("word").and_then(|w| w.as_str()) {
            output.push_str(&format!("**{}**\n", word));
        }
        if let Some(definitions) = self.data.get("definitions").and_then(|d| d.as_array()) {
            for (i, def) in definitions.iter().take(3).enumerate() {
                if let Some(text) = def.get("definition").and_then(|t| t.as_str()) {
                    output.push_str(&format!("{}. {}\n", i + 1, text));
                }
            }
        }

        Some(output)
    }
}

// ============================================================================
// Result Formatting
// ============================================================================

impl SearchResponse {
    pub fn format_results(&self) -> String {
        let mut output = String::new();

        // Show if query was altered (spellcheck)
        if let Some(ref query) = self.query {
            if let Some(ref altered) = query.altered {
                if query.original.as_ref() != Some(altered) {
                    output.push_str(&format!("*Showing results for: {}*\n\n", altered));
                }
            }
        }

        // Rich data first (most specific/useful for intent-based queries)
        if let Some(ref rich) = self.rich_data {
            if let Some(formatted) = rich.format() {
                output.push_str(&formatted);
                output.push_str("\n\n---\n\n");
            }
        }

        // AI summary
        if let Some(ref summary) = self.summary_text {
            output.push_str("**AI Summary:**\n");
            output.push_str(summary);
            output.push_str("\n\n---\n\n");
        }

        // Infobox (quick facts)
        if let Some(infobox) = &self.infobox {
            if let Some(title) = &infobox.title {
                output.push_str(&format!("**{}**\n", title));
                if let Some(desc) = infobox.long_desc.as_ref().or(infobox.description.as_ref()) {
                    output.push_str(&format!("{}\n\n", desc));
                }
            }
        }

        // FAQ (direct answers)
        if let Some(faq) = &self.faq {
            if let Some(results) = &faq.results {
                if !results.is_empty() {
                    output.push_str("**FAQ:**\n\n");
                    for faq_item in results.iter().take(3) {
                        output.push_str(&format!(
                            "Q: {}\nA: {}\n\n",
                            faq_item.question, faq_item.answer
                        ));
                    }
                }
            }
        }

        // Web results
        if let Some(web) = &self.web {
            if let Some(results) = &web.results {
                if !results.is_empty() {
                    output.push_str("**Search Results:**\n\n");
                    for (i, result) in results.iter().take(5).enumerate() {
                        let age = result
                            .age
                            .as_deref()
                            .map(|a| format!(" ({})", a))
                            .unwrap_or_default();
                        output.push_str(&format!(
                            "{}. {}{}\n   URL: {}\n   {}\n",
                            i + 1,
                            result.title,
                            age,
                            result.url,
                            result.description.as_deref().unwrap_or("")
                        ));
                        if let Some(extras) = &result.extra_snippets {
                            for snippet in extras.iter().take(2) {
                                output.push_str(&format!("   > {}\n", snippet));
                            }
                        }
                        output.push('\n');
                    }
                }
            }
        }

        // News
        if let Some(news) = &self.news {
            if let Some(results) = &news.results {
                if !results.is_empty() {
                    output.push_str("**Recent News:**\n\n");
                    for (i, result) in results.iter().take(3).enumerate() {
                        let age = result
                            .age
                            .as_deref()
                            .map(|a| format!(" ({})", a))
                            .unwrap_or_default();
                        output.push_str(&format!(
                            "{}. {}{}\n   URL: {}\n   {}\n\n",
                            i + 1,
                            result.title,
                            age,
                            result.url,
                            result.description.as_deref().unwrap_or("")
                        ));
                    }
                }
            }
        }

        // Discussions
        if let Some(discussions) = &self.discussions {
            if let Some(results) = &discussions.results {
                if !results.is_empty() {
                    output.push_str("**Discussions:**\n\n");
                    for result in results.iter().take(2) {
                        output.push_str(&format!("- {}\n  {}\n\n", result.title, result.url,));
                    }
                }
            }
        }

        if output.is_empty() {
            "No results found.".to_string()
        } else {
            output
        }
    }
}
