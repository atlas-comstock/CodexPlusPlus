use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_AD_LIST_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
    "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
];

fn ad_text_field(ad: &Value) -> Option<&str> {
    ad.get("adText")
        .or_else(|| ad.get("text"))
        .and_then(Value::as_str)
}

fn is_valid_ad(ad: &Value) -> bool {
    match ad.get("type").and_then(Value::as_str) {
        Some("text") => ad_text_field(ad).is_some_and(|value| !value.trim().is_empty()),
        Some("sponsor" | "normal") => {
            let title = ad.get("title").and_then(Value::as_str);
            let description = ad.get("description").and_then(Value::as_str);
            let url = ad.get("url").and_then(Value::as_str);
            title.is_some_and(|value| !value.trim().is_empty())
                && description.is_some_and(|value| !value.trim().is_empty())
                && url.is_some_and(|value| !value.trim().is_empty())
        }
        _ => false,
    }
}

pub fn ad_display_text(ad: &Value) -> String {
    if let Some(text) = ad_text_field(ad) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let title = ad
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let description = ad
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !title.is_empty() && !description.is_empty() {
        format!("{title} — {description}")
    } else if !description.is_empty() {
        description.to_string()
    } else {
        title.to_string()
    }
}

pub fn enrich_ad_for_client(ad: Value) -> Value {
    let ad_text = ad_display_text(&ad);
    let mut enriched = ad;
    if let Some(obj) = enriched.as_object_mut() {
        obj.insert("adText".to_string(), json!(ad_text));
    }
    enriched
}

pub fn ad_fetch_response(nonce: String, ad: Value) -> Value {
    let ad = enrich_ad_for_client(ad);
    let ad_text = ad.get("adText").cloned().unwrap_or_else(|| json!(""));
    json!({ "nonce": nonce, "ad": ad, "adText": ad_text })
}

pub fn normalize_ad_payload(payload: Value) -> Value {
    let version = payload.get("version").and_then(Value::as_u64).unwrap_or(1);
    let ads = payload
        .get("ads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|ad| is_valid_ad(ad))
        .map(|ad| {
            if ad.get("type").and_then(Value::as_str) == Some("text") {
                let mut normalized = ad.clone();
                if let Some(obj) = normalized.as_object_mut() {
                    if let Some(text) = ad_text_field(&ad) {
                        obj.insert("adText".to_string(), json!(text.trim()));
                    }
                    if !obj.contains_key("url") {
                        obj.insert("url".to_string(), json!("#"));
                    }
                }
                normalized
            } else {
                ad.clone()
            }
        })
        .collect::<Vec<_>>();
    json!({ "version": version, "ads": ads })
}

pub async fn fetch_ad_list() -> anyhow::Result<Value> {
    fetch_ad_list_from_urls(&DEFAULT_AD_LIST_URLS).await
}

pub fn pick_random_ad(payload: &Value) -> Option<Value> {
    let ads = payload.get("ads").and_then(Value::as_array)?;
    if ads.is_empty() {
        return None;
    }
    let idx = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as usize)
        .unwrap_or(0)
        % ads.len();
    Some(ads[idx].clone())
}

pub async fn fetch_random_ad() -> anyhow::Result<Value> {
    let list = fetch_ad_list().await?;
    pick_random_ad(&list).ok_or_else(|| anyhow::anyhow!("no ads available"))
}

pub fn cache_busted_ad_url(url: &str, version: u128) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}v={version}")
}

pub async fn fetch_ad_list_from_urls<S>(urls: &[S]) -> anyhow::Result<Value>
where
    S: AsRef<str>,
{
    let client = crate::http_client::proxied_client("CodexPlusPlus")?;
    let cache_bust = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let mut last_error = None;
    for url in urls {
        let url = cache_busted_ad_url(url.as_ref(), cache_bust);
        let result = async {
            let response = client.get(url).send().await?.error_for_status()?;
            let payload = response.json::<Value>().await?;
            Ok::<_, anyhow::Error>(normalize_ad_payload(payload))
        }
        .await;
        match result {
            Ok(payload) => return Ok(payload),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("ad list unavailable")))
}
