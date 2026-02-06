//! Vision Pre-Processing
//!
//! Describes images sent via Signal by calling a vision-capable LLM (Kimi K2.5)
//! directly via the OpenAI-compatible API. The resulting description is injected
//! into the conversation as text alongside the user's message.

use anyhow::{Context, Result};
use base64::Engine;
use tracing::{debug, info, warn};

/// Describes an image using a vision-capable model via the OpenAI-compatible API.
///
/// `recent_messages` should contain the last few user/assistant turns for context
/// (formatted as simple "[role]: content" lines).
pub async fn describe_image(
    api_url: &str,
    api_key: &str,
    model: &str,
    image_path: &str,
    content_type: &str,
    user_message: &str,
    recent_messages: &str,
) -> Result<String> {
    let image_data = std::fs::read(image_path)
        .with_context(|| format!("Failed to read image file: {}", image_path))?;
    let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_data);
    let data_url = format!("data:{};base64,{}", content_type, base64_image);

    info!(
        "Describing image ({}, {} bytes) with model {}",
        content_type,
        image_data.len(),
        model
    );

    let system_prompt = "You are an image description agent. Your ONLY job is to describe the \
        image the user sent in extreme detail with as much accuracy as possible. \
        Describe everything you see: objects, people, text, colors, layout, \
        emotions, context, setting, lighting, and any other relevant details. \
        Be thorough but organized. If there is text in the image, transcribe it exactly. \
        Recent conversation context is provided so you can understand what the user \
        might be referring to - use it to make your description more relevant, \
        but your primary job is accurate visual description. \
        Output ONLY the description, nothing else.";

    let mut user_content = Vec::new();

    // Add the image
    user_content.push(serde_json::json!({
        "type": "image_url",
        "image_url": { "url": data_url }
    }));

    // Build text prompt with context
    let mut text_parts = Vec::new();
    if !recent_messages.is_empty() {
        text_parts.push(format!(
            "Recent conversation for context:\n{}",
            recent_messages
        ));
    }
    if !user_message.is_empty() {
        text_parts.push(format!(
            "The user sent this message alongside the image: \"{}\"",
            user_message
        ));
    }
    text_parts.push("Describe this image in detail.".to_string());

    user_content.push(serde_json::json!({
        "type": "text",
        "text": text_parts.join("\n\n")
    }));

    let request_body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_content }
        ],
        "max_tokens": 2048,
    });

    debug!("Vision API request to {}/chat/completions", api_url);

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/chat/completions", api_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .context("Failed to call vision API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        warn!("Vision API error {}: {}", status, body);
        anyhow::bail!("Vision API returned {}: {}", status, body);
    }

    let json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse vision API response")?;
    let description = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("[Could not describe image]")
        .to_string();

    info!("Image described successfully ({} chars)", description.len());
    debug!(
        "Image description: {}",
        &description[..description.len().min(200)]
    );

    Ok(description)
}

/// Check if a MIME type is an image type we can process
pub fn is_supported_image(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/webp" | "image/gif"
    )
}
