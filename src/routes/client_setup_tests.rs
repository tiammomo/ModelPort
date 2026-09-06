use axum::{
    body::Body,
    http::{Request, StatusCode, header::COOKIE},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{
    tests::{create_test_api_key, get_console_json, login_cookie, test_state_with_admin},
    *,
};
use crate::{auth::CreateUserInput, governance::ChangeRequestInput};

fn fixture(base_url: &str) -> (AppState, crate::control::CreatedApiKey) {
    let state = test_state_with_admin(base_url.to_owned(), 1024 * 1024);
    let user = state
        .auth
        .create_user(CreateUserInput {
            username: "setup-member".to_owned(),
            email: "member@example.test".to_owned(),
            password: "member-password-123".to_owned(),
            role: Some("user".to_owned()),
            status: Some("active".to_owned()),
        })
        .unwrap();
    let key = create_test_api_key(&state, &user, "setup key");
    (state, key)
}

fn allow_cloud(state: &AppState, classification: &str) {
    let change = state.governance.create_change_request("admin", "admin", ChangeRequestInput {
        action: "project_policy.upsert".to_owned(),
        target: "org_local/prj_default/env_default".to_owned(),
        payload: json!({
            "organizationId": "org_local", "projectId": "prj_default", "environmentId": "env_default",
            "maximumMode": "cloud_first", "defaultClassification": classification,
            "allowedProviders": ["mimo"], "allowedModels": ["mimo-v2.5-pro"],
            "allowedRegions": ["global"], "allowedApiVersions": ["openai-compatible-v1"], "cloudEnabled": true,
        }),
        reason: "reviewed cloud setup fixture".to_owned(),
    }).unwrap();
    state
        .governance
        .apply_project_policy(&change.id, "admin", false)
        .unwrap();
}

async fn setup(app: Router, key_id: &str) -> Value {
    let cookie = login_cookie(app.clone(), "admin", "strong-password-123").await;
    get_console_json(
        app,
        &format!("/admin/api-keys/{key_id}/setup?model=mimo-v2.5-pro"),
        cookie,
    )
    .await
}

#[tokio::test]
async fn setup_blocks_unclassified_cloud_until_project_explicitly_classifies_data() {
    let (state, key) = fixture("https://provider.example/v1");
    let app = router(state.clone());
    assert_eq!(
        setup(app.clone(), &key.public.id).await["code"],
        "project_policy_denied"
    );
    allow_cloud(&state, "unknown");
    let result = setup(app.clone(), &key.public.id).await;
    assert_eq!(result["status"], "blocked");
    assert_eq!(result["code"], "local_only");
    assert_eq!(result["defaultClassification"], "unknown");
    allow_cloud(&state, "internal");
    let result = setup(app.clone(), &key.public.id).await;
    assert_eq!(result["status"], "ready");
    assert_eq!(result["upstreamVerified"], false);
    assert!(state.ledger.usage_rows().await.unwrap().is_empty());
    allow_cloud(&state, "sensitive");
    assert_eq!(setup(app, &key.public.id).await["code"], "local_only");
}

#[tokio::test]
async fn setup_handles_local_defaults_revocation_and_disabled_owner_without_upstream() {
    let (state, key) = fixture("http://127.0.0.1:9/v1");
    let app = router(state.clone());
    assert_eq!(setup(app.clone(), &key.public.id).await["status"], "ready");
    state.control.revoke_api_key(&key.public.id).unwrap();
    assert_eq!(
        setup(app.clone(), &key.public.id).await["code"],
        "key_unavailable"
    );
    state
        .auth
        .update_user(
            &key.public.user_id,
            "admin",
            serde_json::from_value(json!({"status": "disabled"})).unwrap(),
        )
        .unwrap();
    assert_eq!(
        setup(app, &key.public.id).await["code"],
        "owner_unavailable"
    );
}

#[tokio::test]
async fn setup_is_owner_scoped_uncached_and_requires_a_console_session() {
    let (state, key) = fixture("http://127.0.0.1:9/v1");
    state
        .auth
        .create_user(CreateUserInput {
            username: "outsider".to_owned(),
            email: "outsider@example.test".to_owned(),
            password: "outsider-password-123".to_owned(),
            role: Some("user".to_owned()),
            status: Some("active".to_owned()),
        })
        .unwrap();
    let app = router(state);
    let uri = format!(
        "/admin/api-keys/{}/setup?model=mimo-v2.5-pro",
        key.public.id
    );
    let response = app
        .clone()
        .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for (name, password, expected) in [
        ("outsider", "outsider-password-123", StatusCode::FORBIDDEN),
        ("setup-member", "member-password-123", StatusCode::OK),
    ] {
        let cookie = login_cookie(app.clone(), name, password).await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&uri)
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}

#[tokio::test]
async fn administrator_key_catalog_and_setup_honor_key_model_restrictions() {
    let (state, key) = fixture("http://127.0.0.1:9/v1");
    state
        .control
        .update_api_key(
            &key.public.id,
            serde_json::from_value(json!({"allowedModels": ["different-model"]})).unwrap(),
        )
        .unwrap();
    let app = router(state);
    let cookie = login_cookie(app.clone(), "admin", "strong-password-123").await;
    let global = get_console_json(app.clone(), "/admin/providers", cookie.clone()).await;
    assert!(!global.as_array().unwrap().is_empty());
    for endpoint in ["providers", "aliases"] {
        let rows = get_console_json(
            app.clone(),
            &format!("/admin/{endpoint}?apiKeyId={}", key.public.id),
            cookie.clone(),
        )
        .await;
        assert_eq!(rows, json!([]));
    }
    assert_eq!(
        setup(app, &key.public.id).await["code"],
        "model_not_allowed"
    );
}

#[tokio::test]
async fn setup_rejects_operations_control_keys_for_inference() {
    let (state, owner_key) = fixture("http://127.0.0.1:9/v1");
    let key = state
        .control
        .create_api_key(
            serde_json::from_value(json!({
                "userId": owner_key.public.user_id,
                "name": "operations control",
                "principalType": "service_account",
                "purpose": "modelport_ops_agent",
                "allowedProviders": ["mimo"],
                "allowedModels": ["mimo-v2.5-pro"],
                "expiresAt": (now_millis() + 3_600_000).to_string(),
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        setup(router(state), &key.public.id).await["code"],
        "control_only_key"
    );
}
