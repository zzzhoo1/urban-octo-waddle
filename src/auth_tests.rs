use crate::auth::*;

#[test]
fn master_key_authenticates() {
    let auth = KeyAuth::new("secret", vec!["master-secret".to_string()]);
    assert!(auth.authenticate("master-secret"));
    let key = auth.get_by_name("master").unwrap();
    assert!(key.is_master);
    assert_eq!(key.role, KeyRole::Admin);
}

#[test]
fn unknown_key_fails() {
    let auth = KeyAuth::new("secret", vec!["master-secret".to_string()]);
    assert!(!auth.authenticate("wrong"));
    assert_eq!(
        auth.authenticate_for_model("wrong", "m", "m"),
        Err(AuthFailure::UnknownKey)
    );
}

#[test]
fn data_plane_ignores_role() {
    let user = make_test_key("u", "user-key", KeyRole::User);
    let auth = KeyAuth::from_entries("secret", vec![user]);
    assert!(auth.authenticate_for_model("user-key", "m", "m").is_ok());
    assert_eq!(
        auth.authenticate_admin("user-key"),
        Err(AuthFailure::Forbidden)
    );
}

#[test]
fn admin_role_passes_admin_auth() {
    let admin = make_test_key("a", "admin-key", KeyRole::Admin);
    let auth = KeyAuth::from_entries("secret", vec![admin]);
    assert!(auth.authenticate_admin("admin-key").is_ok());
}

#[test]
fn disabled_key_rejected() {
    let mut k = make_test_key("d", "disabled-key", KeyRole::User);
    k.enabled = false;
    let auth = KeyAuth::from_entries("secret", vec![k]);
    assert_eq!(
        auth.authenticate_for_model("disabled-key", "m", "m"),
        Err(AuthFailure::Disabled)
    );
}

#[test]
fn expired_key_rejected() {
    let mut k = make_test_key("e", "expired-key", KeyRole::User);
    k.expires_at = Some(chrono::Utc::now() - chrono::Duration::hours(1));
    let auth = KeyAuth::from_entries("secret", vec![k]);
    assert_eq!(
        auth.authenticate_for_model("expired-key", "m", "m"),
        Err(AuthFailure::Expired)
    );
}

#[test]
fn quota_exceeded_rejected() {
    let mut k = make_test_key("q", "quota-key", KeyRole::User);
    k.token_quota = 10;
    k.tokens_used = 10;
    let auth = KeyAuth::from_entries("secret", vec![k]);
    assert_eq!(
        auth.authenticate_for_model("quota-key", "m", "m"),
        Err(AuthFailure::QuotaExceeded)
    );
}

#[test]
fn model_allowlist_enforced() {
    let mut k = make_test_key("w", "wl-key", KeyRole::User);
    k.model_allowlist = vec!["allowed-model".to_string()];
    let auth = KeyAuth::from_entries("secret", vec![k]);
    assert!(auth
        .authenticate_for_model("wl-key", "allowed-model", "allowed-model")
        .is_ok());
    assert_eq!(
        auth.authenticate_for_model("wl-key", "other", "other"),
        Err(AuthFailure::ModelNotAllowed)
    );
}

#[test]
fn record_usage_accumulates() {
    let k = make_test_key("r", "usage-key", KeyRole::User);
    let fp = k.fingerprint.clone();
    let auth = KeyAuth::from_entries("secret", vec![k]);
    auth.record_usage(&fp, 5);
    auth.record_usage(&fp, 7);
    let key = auth.get_by_name("r").unwrap();
    assert_eq!(key.tokens_used, 12);
    assert_eq!(key.tokens_used_in_period, 12);
}

#[test]
fn daily_quota_rolls() {
    let mut k = make_test_key("d", "daily-key", KeyRole::User);
    k.quota_period = QuotaPeriod::Daily;
    k.quota_period_start = Some(chrono::Utc::now() - chrono::Duration::days(2));
    k.tokens_used_in_period = 100;
    assert!(k.roll_quota_period(chrono::Utc::now()));
    assert_eq!(k.tokens_used_in_period, 0);
}

#[test]
fn legacy_key_lazy_upgrades() {
    let mut k = make_test_key("l", "legacy-key", KeyRole::User);
    k.fingerprint = String::new();
    k.key_hash = hash_key("legacy-key");
    k.legacy_hash = true;
    let auth = KeyAuth::from_entries("secret", vec![k]);
    assert!(auth.authenticate("legacy-key"));
    auth.lazy_upgrade("legacy-key");
    let key = auth.get_by_name("l").unwrap();
    assert!(!key.legacy_hash);
    assert!(!key.fingerprint.is_empty());
}

fn make_test_key(name: &str, plaintext: &str, role: KeyRole) -> ApiKey {
    ApiKey {
        name: name.to_string(),
        key_hash: hash_key_slow(plaintext),
        fingerprint: fingerprint_key("secret", plaintext),
        hint: mask_key(plaintext),
        created_at: Some(chrono::Utc::now()),
        expires_at: None,
        token_quota: 0,
        tokens_used: 0,
        model_allowlist: Vec::new(),
        enabled: true,
        role,
        is_master: false,
        legacy_hash: false,
        quota_period: QuotaPeriod::None,
        quota_period_start: None,
        tokens_used_in_period: 0,
    }
}

#[test]
fn extract_api_key_prefers_bearer() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer abc"),
    );
    headers.insert(
        "X-Api-Key",
        axum::http::HeaderValue::from_static("xyz"),
    );
    assert_eq!(extract_api_key(&headers), "abc");
}

#[test]
fn extract_api_key_falls_back_to_x_api_key() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-Api-Key", axum::http::HeaderValue::from_static("xyz"));
    assert_eq!(extract_api_key(&headers), "xyz");
}