use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

static NONCE_COUNTER: AtomicUsize = AtomicUsize::new(0);
static VALID_NONCES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

pub const REWARD_PER_AD: i64 = 10;
pub const COST_PER_REQUEST: i64 = 0;
pub const INITIAL_BALANCE: i64 = 100;
pub const RECENT_EVENT_LIMIT: usize = 30;

fn nonces() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    VALID_NONCES.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

pub fn generate_ad_nonce() -> String {
    let count = NONCE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let nonce = format!("fc-nonce-{}-{}", time, count);
    if let Ok(mut set) = nonces().lock() {
        set.insert(nonce.clone());
    }
    nonce
}

pub fn verify_and_consume_nonce(nonce: &str) -> bool {
    if let Ok(mut set) = nonces().lock() {
        set.remove(nonce)
    } else {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditEventKind {
    Earn,
    Spend,
    Impression,
    Click,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditEvent {
    pub kind: CreditEventKind,
    pub amount: i64,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub note: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditsState {
    pub balance: i64,
    #[serde(default)]
    pub total_earned: i64,
    #[serde(default)]
    pub total_spent: i64,
    #[serde(default)]
    pub ad_reward_count: u64,
    #[serde(default)]
    pub waiting_bar_rewards: u64,
    #[serde(default)]
    pub modal_rewards: u64,
    #[serde(default)]
    pub ad_impression_count: u64,
    #[serde(default)]
    pub ad_click_count: u64,
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub recent_events: Vec<CreditEvent>,
}

impl Default for CreditsState {
    fn default() -> Self {
        Self {
            balance: INITIAL_BALANCE,
            total_earned: 0,
            total_spent: 0,
            ad_reward_count: 0,
            waiting_bar_rewards: 0,
            modal_rewards: 0,
            ad_impression_count: 0,
            ad_click_count: 0,
            request_count: 0,
            recent_events: Vec::new(),
        }
    }
}

impl CreditsState {
    fn push_event(&mut self, event: CreditEvent) {
        self.recent_events.insert(0, event);
        self.recent_events.truncate(RECENT_EVENT_LIMIT);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_surface(surface: &str) -> &'static str {
    match surface.trim().to_ascii_lowercase().as_str() {
        "waiting" | "waiting_bar" | "waiting-bar" | "thinking" => "waiting_bar",
        "modal" | "manual" => "modal",
        "request" | "chat" => "request",
        _ => "unknown",
    }
}

pub async fn get_credits_file() -> anyhow::Result<PathBuf> {
    let mut path = crate::relay_config::default_codex_home_dir();
    path.push("codex-plus-credits.json");
    Ok(path)
}

pub async fn load_credits_state() -> anyhow::Result<CreditsState> {
    let path = get_credits_file().await?;
    if !path.exists() {
        let default = CreditsState::default();
        save_credits_state(&default).await?;
        return Ok(default);
    }
    let content = fs::read_to_string(&path).await?;
    let state = match serde_json::from_str::<CreditsState>(&content) {
        Ok(parsed) => parsed,
        Err(_) => {
            #[derive(Deserialize)]
            struct LegacyCredits {
                balance: i64,
            }
            serde_json::from_str::<LegacyCredits>(&content)
                .map(|legacy| CreditsState {
                    balance: legacy.balance.max(0),
                    ..CreditsState::default()
                })
                .unwrap_or_default()
        }
    };
    Ok(state)
}

pub async fn save_credits_state(state: &CreditsState) -> anyhow::Result<()> {
    let path = get_credits_file().await?;
    fs::write(&path, serde_json::to_string_pretty(state)?).await?;
    Ok(())
}

pub async fn load_credits() -> anyhow::Result<CreditsState> {
    load_credits_state().await
}

pub async fn add_credits(amount: i64) -> anyhow::Result<i64> {
    let mut state = load_credits_state().await.unwrap_or_default();
    state.balance += amount;
    let _ = save_credits_state(&state).await;
    Ok(state.balance)
}

pub async fn record_ad_reward(surface: &str, amount: i64) -> anyhow::Result<CreditsState> {
    let mut state = load_credits_state().await?;
    let normalized = normalize_surface(surface);
    state.balance += amount;
    state.total_earned += amount;
    state.ad_reward_count += 1;
    match normalized {
        "waiting_bar" => state.waiting_bar_rewards += 1,
        "modal" => state.modal_rewards += 1,
        _ => {}
    }
    state.push_event(CreditEvent {
        kind: CreditEventKind::Earn,
        amount,
        surface: normalized.to_string(),
        note: "看广告奖励".to_string(),
        at_ms: now_ms(),
    });
    save_credits_state(&state).await?;
    Ok(state)
}

pub async fn record_ad_impression(surface: &str) -> anyhow::Result<()> {
    let mut state = load_credits_state().await?;
    let normalized = normalize_surface(surface);
    state.ad_impression_count += 1;
    state.push_event(CreditEvent {
        kind: CreditEventKind::Impression,
        amount: 0,
        surface: normalized.to_string(),
        note: "广告展示".to_string(),
        at_ms: now_ms(),
    });
    save_credits_state(&state).await
}

pub async fn record_ad_click(surface: &str) -> anyhow::Result<()> {
    let mut state = load_credits_state().await?;
    let normalized = normalize_surface(surface);
    state.ad_click_count += 1;
    state.push_event(CreditEvent {
        kind: CreditEventKind::Click,
        amount: 0,
        surface: normalized.to_string(),
        note: "广告点击".to_string(),
        at_ms: now_ms(),
    });
    save_credits_state(&state).await
}

pub async fn deduct_credits(amount: i64) -> anyhow::Result<bool> {
    if amount <= 0 {
        return Ok(true);
    }
    let mut state = load_credits_state().await.unwrap_or_default();
    if state.balance >= amount {
        state.balance -= amount;
        state.total_spent += amount;
        state.request_count += 1;
        state.push_event(CreditEvent {
            kind: CreditEventKind::Spend,
            amount,
            surface: "request".to_string(),
            note: "AI 对话消耗".to_string(),
            at_ms: now_ms(),
        });
        let _ = save_credits_state(&state).await;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn credits_summary(state: &CreditsState) -> Value {
    let phase = if COST_PER_REQUEST <= 0 {
        "free"
    } else {
        "metered"
    };
    json!({
        "balance": state.balance,
        "reward_per_ad": REWARD_PER_AD,
        "cost_per_request": COST_PER_REQUEST,
        "phase": phase,
        "initial_balance": INITIAL_BALANCE,
        "stats": {
            "total_earned": state.total_earned,
            "total_spent": state.total_spent,
            "ad_reward_count": state.ad_reward_count,
            "waiting_bar_rewards": state.waiting_bar_rewards,
            "modal_rewards": state.modal_rewards,
            "ad_impression_count": state.ad_impression_count,
            "ad_click_count": state.ad_click_count,
            "request_count": state.request_count,
            "recent_events": state.recent_events,
        },
        "help": {
            "title": "算力（Compute Credits）说明",
            "earn": format!(
                "看广告赚算力：等待栏广告满 5 秒或手动看广告，每次 +{} 算力。",
                REWARD_PER_AD
            ),
            "spend": if COST_PER_REQUEST > 0 {
                format!("每次 AI 对话消耗 {} 算力。", COST_PER_REQUEST)
            } else {
                "当前为全免费阶段：每次 AI 对话暂不扣算力，余额不会因对话减少。".to_string()
            },
            "future": "算力不足时，将自动降级到节流模式；看广告可继续补充算力。",
        }
    })
}

pub async fn credits_get_response() -> anyhow::Result<Value> {
    let state = load_credits_state().await?;
    Ok(credits_summary(&state))
}

pub async fn record_credit_ad_event(event_type: &str, surface: &str) -> anyhow::Result<Value> {
    match event_type.trim().to_ascii_lowercase().as_str() {
        "impression" => {
            record_ad_impression(surface).await?;
        }
        "click" => {
            record_ad_click(surface).await?;
        }
        _ => {}
    }
    credits_get_response().await
}

pub async fn verify_ad_reward(surface: &str, nonce: &str) -> anyhow::Result<Value> {
    if verify_and_consume_nonce(nonce) {
        let state = record_ad_reward(surface, REWARD_PER_AD).await?;
        let summary = credits_summary(&state);
        Ok(json!({
            "success": true,
            "balance": state.balance,
            "reward": REWARD_PER_AD,
            "stats": summary.get("stats").cloned().unwrap_or_else(|| json!({})),
        }))
    } else {
        Ok(json!({
            "success": false,
            "error": "Invalid or expired ad nonce"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn with_test_home<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let home = std::env::temp_dir().join(format!(
            "freecodex-credits-test-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, AtomicOrdering::SeqCst)
        ));
        std::fs::create_dir_all(&home).unwrap();
        let previous = std::env::var("CODEX_HOME").ok();
        unsafe {
            std::env::set_var("CODEX_HOME", &home);
        }
        f().await;
        let _ = std::fs::remove_dir_all(&home);
        if let Some(value) = previous {
            unsafe {
                std::env::set_var("CODEX_HOME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("CODEX_HOME");
            }
        }
    }

    #[tokio::test]
    async fn credits_state_tracks_rewards_and_spends() {
        with_test_home(|| async {
            record_ad_reward("waiting_bar", REWARD_PER_AD)
                .await
                .expect("reward");
            let rewarded = load_credits_state().await.expect("load rewarded");
            assert_eq!(rewarded.balance, INITIAL_BALANCE + REWARD_PER_AD);
            assert_eq!(rewarded.waiting_bar_rewards, 1);
            assert_eq!(rewarded.modal_rewards, 0);
            assert_eq!(rewarded.ad_reward_count, 1);

            let mut spendable = rewarded;
            spendable.balance = 5;
            save_credits_state(&spendable).await.expect("save spendable");
            assert!(deduct_credits(2).await.expect("deduct"));
            let spent = load_credits_state().await.expect("load spent");
            assert_eq!(spent.balance, 3);
            assert_eq!(spent.total_spent, 2);
            assert_eq!(spent.request_count, 1);
        })
        .await;
    }
}