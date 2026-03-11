use crate::settings::{PostProcessProvider, PROVIDER_ID_ANTHROPIC, PROVIDER_ID_GEMINI};
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const REQUEST_TIMEOUT_SECS: u64 = 30;
const PHRASER_USER_AGENT: &str = "Parler/1.0 (+https://github.com/fxbenard/Parler)";
const PHRASER_REFERER: &str = "https://github.com/fxbenard/Parler";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(REFERER, HeaderValue::from_static(PHRASER_REFERER));
    headers.insert(USER_AGENT, HeaderValue::from_static(PHRASER_USER_AGENT));
    headers.insert("X-Title", HeaderValue::from_static("Parler"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == PROVIDER_ID_ANTHROPIC {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert(
                "anthropic-version",
                HeaderValue::from_static(ANTHROPIC_API_VERSION),
            );
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create a base HTTP client builder with shared configuration (timeout, etc.)
fn create_base_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
}

/// Create an HTTP client with provider-specific headers
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    create_base_client()
        .default_headers(headers)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(provider, api_key, model, prompt, None, None).await
}

/// Send a chat completion with a system prompt but no structured output schema.
/// Use this instead of `send_chat_completion_with_schema(..., None)` to make
/// intent explicit at the call site.
pub async fn send_chat_completion_with_system(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: String,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        user_content,
        Some(system_prompt),
        None,
    )
    .await
}

/// Send a chat completion request with structured output support
/// When json_schema is provided, uses structured outputs mode
/// system_prompt is used as the system message when provided
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
) -> Result<Option<String>, String> {
    // Route Gemini requests to the dedicated Gemini client
    if provider.id == PROVIDER_ID_GEMINI {
        let sys = system_prompt.unwrap_or_default();
        match crate::gemini_client::generate_text(&api_key, model, &sys, &user_content).await {
            Ok(text) if !text.is_empty() => return Ok(Some(text)),
            Ok(_) => return Ok(None),
            Err(e) => return Err(format!("Gemini API error: {}", e)),
        }
    }

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending chat completion request to: {}", url);

    let client = create_client(provider, &api_key)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }

    // Add user message
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format,
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone()))
}

async fn fetch_gemini_models(api_key: &str) -> Result<Vec<String>, String> {
    let url = "https://generativelanguage.googleapis.com/v1beta/models";

    let client = create_base_client()
        .build()
        .map_err(|e| format!("Failed to build Gemini HTTP client: {}", e))?;

    let response = client
        .get(url)
        .header("x-goog-api-key", api_key)
        .header(USER_AGENT, PHRASER_USER_AGENT)
        .header(REFERER, PHRASER_REFERER)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Gemini models: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Gemini model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

    let mut models = Vec::new();
    if let Some(data) = parsed.get("models").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                // Gemini returns "models/gemini-2.5-flash" - strip the prefix
                let model_id = name.strip_prefix("models/").unwrap_or(name);
                if model_id.contains("gemini") {
                    models.push(model_id.to_string());
                }
            }
        }
    }

    Ok(models)
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    // Gemini uses a different API format for listing models
    if provider.id == PROVIDER_ID_GEMINI {
        return fetch_gemini_models(&api_key).await;
    }

    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", url);

    let client = create_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::PROVIDER_ID_OPENAI;

    fn make_provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url: base_url.to_string(),
            allow_base_url_edit: false,
            requires_api_key: true,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    #[test]
    fn build_headers_common_fields() {
        let provider = make_provider("openai", "https://api.openai.com/v1");
        let headers = build_headers(&provider, "sk-test-key").unwrap();

        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert!(headers.get(USER_AGENT).is_some());
        assert!(headers.get(REFERER).is_some());
    }

    #[test]
    fn build_headers_bearer_auth_for_openai() {
        let provider = make_provider("openai", "https://api.openai.com/v1");
        let headers = build_headers(&provider, "sk-test-key").unwrap();

        assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer sk-test-key");
    }

    #[test]
    fn build_headers_anthropic_uses_x_api_key() {
        let provider = make_provider(PROVIDER_ID_ANTHROPIC, "https://api.anthropic.com/v1");
        let headers = build_headers(&provider, "sk-ant-test").unwrap();

        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        // Should NOT have Bearer auth
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn build_headers_empty_api_key_no_auth() {
        let provider = make_provider("openai", "https://api.openai.com/v1");
        let headers = build_headers(&provider, "").unwrap();

        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn chat_completion_request_serializes() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: "You are helpful.".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            ],
            response_format: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("gpt-4"));
        assert!(json.contains("system"));
        assert!(json.contains("user"));
        // response_format should be absent (skip_serializing_if)
        assert!(!json.contains("response_format"));
    }

    #[test]
    fn chat_completion_request_with_schema_serializes() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        });

        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            response_format: Some(ResponseFormat {
                format_type: "json_schema".to_string(),
                json_schema: JsonSchema {
                    name: "test_output".to_string(),
                    strict: true,
                    schema,
                },
            }),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("json_schema"));
        assert!(json.contains("test_output"));
        assert!(json.contains("strict"));
    }

    #[test]
    fn chat_completion_response_deserializes() {
        let json = r#"{
            "choices": [{
                "message": {
                    "content": "Hello, world!"
                }
            }]
        }"#;

        let response: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices.len(), 1);
        assert_eq!(
            response.choices[0].message.content.as_deref(),
            Some("Hello, world!")
        );
    }

    #[test]
    fn chat_completion_response_no_content() {
        let json = r#"{
            "choices": [{
                "message": {
                    "content": null
                }
            }]
        }"#;

        let response: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        assert!(response.choices[0].message.content.is_none());
    }

    #[test]
    fn chat_completion_response_empty_choices() {
        let json = r#"{"choices": []}"#;
        let response: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        assert!(response.choices.is_empty());
    }

    #[test]
    fn referer_header_matches_constant() {
        let provider = make_provider(PROVIDER_ID_OPENAI, "https://api.openai.com/v1");
        let headers = build_headers(&provider, "key").unwrap();
        assert_eq!(
            headers.get(REFERER).unwrap().to_str().unwrap(),
            PHRASER_REFERER
        );
    }

    #[test]
    fn user_agent_header_matches_constant() {
        let provider = make_provider(PROVIDER_ID_OPENAI, "https://api.openai.com/v1");
        let headers = build_headers(&provider, "key").unwrap();
        assert_eq!(
            headers.get(USER_AGENT).unwrap().to_str().unwrap(),
            PHRASER_USER_AGENT
        );
    }
}
