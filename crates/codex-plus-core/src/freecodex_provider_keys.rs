//! Internal upstream API keys for the FreeCodex model pool.
//!
//! Keys are injected at **build time** from environment variables or
//! `.env.freecodex.local` (gitignored). They are not user-configurable at runtime.

include!(concat!(env!("OUT_DIR"), "/freecodex_provider_keys_gen.rs"));

pub fn opencode_key() -> &'static str {
    OPENCODE_KEY
}

pub fn openrouter_key() -> &'static str {
    OPENROUTER_KEY
}

pub fn nvidia_key() -> &'static str {
    NVIDIA_KEY
}

pub fn provider_key(provider: &str) -> Option<&'static str> {
    let key = match provider {
        "opencode" => opencode_key(),
        "openrouter" => openrouter_key(),
        "nvidia" => nvidia_key(),
        _ => return None,
    };
    if key.trim().is_empty() {
        None
    } else {
        Some(key)
    }
}

pub fn has_any_provider() -> bool {
    provider_key("opencode").is_some()
        || provider_key("openrouter").is_some()
        || provider_key("nvidia").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_provider_keys_are_available_when_configured() {
        if !has_any_provider() {
            return;
        }
        assert!(provider_key("opencode").is_some());
        assert!(provider_key("openrouter").is_some());
        assert!(provider_key("nvidia").is_some());
    }
}