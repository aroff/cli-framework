//! The `/v1/admin/*` administrative HTTP surface (spec 023): publish
//! manifests, read/replace/patch policy documents, read change history and
//! restore a prior version, read/replace assignment rules, and export/import
//! the whole configuration set as a bundle.
//!
//! # Auth layering — the order of `.layer()` calls is load-bearing
//!
//! Every route here requires **both** a valid [`CallerIdentity`] (`401` on
//! missing/invalid — the same mechanism [`super::router::config_service_router`]'s
//! read-only routes already use) **and** a match against the deployment's
//! configured admin rule (`403` if the identity is valid but the rule
//! doesn't match — [`require_admin_role`]).
//!
//! axum/tower's `Router::layer` wraps the *current* router, so each
//! successive `.layer()` call becomes the new **outermost** layer — the
//! *last* `.layer()` called is the *first* to run against an incoming
//! request. [`require_admin_role`] reads claims [`require_caller_identity`]
//! inserts into the request extensions, so identity must run first, which
//! means its layer must be applied **last**:
//!
//! ```text
//! Router::new()
//!     .route(...)                                          // every /v1/admin/* route
//!     .layer(from_fn_with_state(admin_rule, require_admin_role))    // applied first => runs SECOND
//!     .layer(from_fn_with_state(identity, require_caller_identity)) // applied last  => runs FIRST
//! ```
//!
//! Getting this backwards would run the admin-role check before a caller's
//! claims exist at all — a real, subtle authorization bug, not just a
//! stylistic one. `tests/integration/config_service_admin_router.rs` proves
//! the actual, observable behavior this ordering must produce: a
//! missing/invalid token is `401` (not `403`, and not a panic from missing
//! extension data) on every route, and a valid, non-admin token is `403`.

use super::admin_store::PolicyWrite;
use super::bundle::{build_export_tar, extract_bundle_from_tar};
use super::error::{AdminWriteError, PolicyValidationError};
use super::fs_store::FsPolicyStore;
use super::identity::{require_admin_role, require_caller_identity, CallerClaims};
use super::merge_patch::merge_patch;
use super::router::{internal_error, missing_sub_response, require_if_match, subject_from_claims};
use super::state::ConfigServiceState;
use super::types::{AssignmentRule, MutationKind, RuleOperator, StoredPolicy};
use super::validate::{
    assignment_rule_missing_profile_errors, default_rule_not_last_errors, validate_policy_for_write,
};
use crate::config::manifest::ConfigManifest;
use crate::config::StaleAction;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, ETAG};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Build the `/v1/admin/*` sub-router — see the module docs for the auth
/// layering order. Merged into the main router by
/// [`super::router::config_service_router`].
pub(crate) fn admin_router(state: Arc<ConfigServiceState>) -> Router {
    let identity = state.identity.clone();
    let admin_rule = Arc::new(state.admin_rule.clone());

    Router::new()
        .route("/v1/admin/manifest/{app}", put(put_manifest))
        .route(
            "/v1/admin/policy/{app}/{profile}",
            get(get_policy_admin).put(put_policy).patch(patch_policy),
        )
        .route(
            "/v1/admin/policy/{app}/{profile}/history",
            get(get_policy_history),
        )
        .route(
            "/v1/admin/policy/{app}/{profile}/history/{version}/restore",
            post(restore_policy),
        )
        .route(
            "/v1/admin/assignments/{app}",
            get(get_assignments).put(put_assignments),
        )
        .route("/v1/admin/export", get(export_bundle))
        .route("/v1/admin/import", post(import_bundle_handler))
        .layer(axum::middleware::from_fn_with_state(
            admin_rule,
            require_admin_role,
        ))
        .layer(axum::middleware::from_fn_with_state(
            identity,
            require_caller_identity,
        ))
        .with_state(state)
}

// ── Shared response helpers ─────────────────────────────────────────────────

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn validation_error_response(errors: Vec<PolicyValidationError>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": errors.iter().map(|e| e.to_string()).collect::<Vec<_>>()
        })),
    )
        .into_response()
}

fn admin_write_error_response(err: AdminWriteError) -> Response {
    match err {
        AdminWriteError::Conflict { .. } => StatusCode::PRECONDITION_FAILED.into_response(),
        AdminWriteError::Validation(errors) => validation_error_response(errors),
        AdminWriteError::Store(e) => internal_error("admin write", e),
    }
}

/// Every `/v1/admin/*` write/read handler that needs [`PolicyAdminStore`]
/// calls this first — `None` means the deployment constructed its
/// [`ConfigServiceState`] without [`ConfigServiceState::with_admin_store`]
/// (an [`super::fs_store::FsPolicyStore`]-only test/dev setup, or simply a
/// deployment-configuration gap), which is a server-side problem, not a
/// client error.
fn admin_store_not_configured() -> Response {
    internal_error(
        "admin write",
        "no PolicyAdminStore configured for this ConfigServiceState",
    )
}

fn etag_response(version: u64) -> Response {
    (StatusCode::OK, [(ETAG, format!("\"{version}\""))]).into_response()
}

fn stored_policy_from_write(
    app: &str,
    profile: &str,
    w: &PolicyWrite,
    version: u64,
) -> StoredPolicy {
    StoredPolicy {
        app: app.to_string(),
        profile: profile.to_string(),
        enforced: w.enforced.clone(),
        recommended: w.recommended.clone(),
        parent_profile: w.parent_profile.clone(),
        max_cache_age_secs: w.max_cache_age_secs,
        stale_action: w.stale_action,
        version,
    }
}

// ── Manifest ─────────────────────────────────────────────────────────────────

async fn put_manifest(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
    Json(raw): Json<Value>,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let expected_version = match require_if_match(&headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let doc: ConfigManifest = match serde_json::from_value(raw) {
        Ok(d) => d,
        Err(e) => return bad_request(format!("request body is not a valid manifest: {e}")),
    };

    match admin_store
        .put_manifest(&app, doc, &actor, expected_version)
        .await
    {
        Ok(v) => etag_response(v),
        Err(e) => admin_write_error_response(e),
    }
}

// ── Policy: GET / PUT / PATCH ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct PolicyDocumentResponse {
    app: String,
    profile: String,
    enforced: Map<String, Value>,
    recommended: Map<String, Value>,
    parent_profile: Option<String>,
    max_cache_age_secs: u64,
    stale_action: StaleAction,
    version: u64,
}

impl From<&StoredPolicy> for PolicyDocumentResponse {
    fn from(p: &StoredPolicy) -> Self {
        Self {
            app: p.app.clone(),
            profile: p.profile.clone(),
            enforced: p.enforced.clone(),
            recommended: p.recommended.clone(),
            parent_profile: p.parent_profile.clone(),
            max_cache_age_secs: p.max_cache_age_secs,
            stale_action: p.stale_action,
            version: p.version,
        }
    }
}

async fn get_policy_admin(
    State(state): State<Arc<ConfigServiceState>>,
    Path((app, profile)): Path<(String, String)>,
    CallerClaims(_claims): CallerClaims,
) -> Response {
    match state.policy_store.policy(&app, &profile).await {
        Ok(Some(p)) => {
            let etag = format!("\"{}\"", p.version);
            let body = PolicyDocumentResponse::from(&p);
            (StatusCode::OK, [(ETAG, etag)], Json(body)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error("admin policy lookup", e),
    }
}

/// The `PUT /v1/admin/policy/{app}/{profile}` request body: every
/// [`PolicyWrite`] field, required (this is a full replace) except
/// `parent_profile`, which defaults to `None` when absent.
#[derive(Debug, Deserialize)]
struct PolicyWriteBody {
    #[serde(default)]
    enforced: Map<String, Value>,
    #[serde(default)]
    recommended: Map<String, Value>,
    #[serde(default)]
    parent_profile: Option<String>,
    max_cache_age_secs: u64,
    stale_action: StaleAction,
}

async fn put_policy(
    State(state): State<Arc<ConfigServiceState>>,
    Path((app, profile)): Path<(String, String)>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
    Json(raw): Json<Value>,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let expected_version = match require_if_match(&headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let body: PolicyWriteBody = match serde_json::from_value(raw.clone()) {
        Ok(b) => b,
        Err(e) => return bad_request(format!("request body is not a valid policy: {e}")),
    };

    let policy_write = PolicyWrite {
        enforced: body.enforced,
        recommended: body.recommended,
        parent_profile: body.parent_profile,
        max_cache_age_secs: body.max_cache_age_secs,
        stale_action: body.stale_action,
    };

    let candidate = stored_policy_from_write(&app, &profile, &policy_write, 0);
    let errors = validate_policy_for_write(state.policy_store.as_ref(), &candidate).await;
    if !errors.is_empty() {
        return validation_error_response(errors);
    }

    match admin_store
        .put_policy(
            &app,
            &profile,
            policy_write,
            MutationKind::PolicyPut,
            raw,
            &actor,
            expected_version,
        )
        .await
    {
        Ok(v) => etag_response(v),
        Err(e) => admin_write_error_response(e),
    }
}

async fn patch_policy(
    State(state): State<Arc<ConfigServiceState>>,
    Path((app, profile)): Path<(String, String)>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
    Json(raw): Json<Value>,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let expected_version = match require_if_match(&headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let Value::Object(patch_obj) = &raw else {
        return bad_request("request body must be a JSON object");
    };

    let current = match state.policy_store.policy(&app, &profile).await {
        Ok(Some(p)) => p,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error("admin policy lookup for patch", e),
    };

    // `enforced`/`recommended` addressed as two independent RFC 7386
    // merge-patch fragments (spec 023 §5) -- this is what lets one request
    // move a field between the two trees.
    let mut enforced_value = Value::Object(current.enforced.clone());
    if let Some(fragment) = patch_obj.get("enforced") {
        merge_patch(&mut enforced_value, fragment);
    }
    let mut recommended_value = Value::Object(current.recommended.clone());
    if let Some(fragment) = patch_obj.get("recommended") {
        merge_patch(&mut recommended_value, fragment);
    }
    let (Value::Object(enforced), Value::Object(recommended)) = (enforced_value, recommended_value)
    else {
        // Unreachable: `merge_patch` only replaces the target wholesale when
        // the patch itself isn't an object, and `current.enforced`/
        // `current.recommended` both start as `Value::Object`, so this can
        // only happen if `"enforced"`/`"recommended"` in the request body is
        // itself present but not an object -- handled as a 400 for honesty,
        // not left as a panic.
        return bad_request("'enforced' and 'recommended' must merge-patch to JSON objects");
    };

    let parent_profile = match patch_obj.get("parent_profile") {
        None => current.parent_profile.clone(),
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return bad_request("'parent_profile' must be a string or null"),
    };

    let max_cache_age_secs = match patch_obj.get("max_cache_age_secs") {
        None => current.max_cache_age_secs,
        Some(v) => match v.as_u64() {
            Some(n) => n,
            None => return bad_request("'max_cache_age_secs' must be a non-negative integer"),
        },
    };

    let stale_action = match patch_obj.get("stale_action") {
        None => current.stale_action,
        Some(Value::String(s)) if s.as_str() == "warn" => StaleAction::Warn,
        Some(Value::String(s)) if s.as_str() == "refuse" => StaleAction::Refuse,
        Some(_) => return bad_request("'stale_action' must be \"warn\" or \"refuse\""),
    };

    let policy_write = PolicyWrite {
        enforced,
        recommended,
        parent_profile,
        max_cache_age_secs,
        stale_action,
    };

    let candidate = stored_policy_from_write(&app, &profile, &policy_write, 0);
    let errors = validate_policy_for_write(state.policy_store.as_ref(), &candidate).await;
    if !errors.is_empty() {
        return validation_error_response(errors);
    }

    match admin_store
        .put_policy(
            &app,
            &profile,
            policy_write,
            MutationKind::PolicyPatch,
            raw,
            &actor,
            expected_version,
        )
        .await
    {
        Ok(v) => etag_response(v),
        Err(e) => admin_write_error_response(e),
    }
}

// ── Policy history + restore ────────────────────────────────────────────────

async fn get_policy_history(
    State(state): State<Arc<ConfigServiceState>>,
    Path((app, profile)): Path<(String, String)>,
    CallerClaims(_claims): CallerClaims,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    match admin_store.policy_history(&app, &profile).await {
        Ok(entries) => (StatusCode::OK, Json(json!({ "entries": entries }))).into_response(),
        Err(e) => internal_error("policy history lookup", e),
    }
}

/// Reconstruct a [`PolicyWrite`] from a `mutation_log.resulting_document`
/// value (the shape [`super::postgres::PgPolicyStore::put_policy`]'s own
/// `policy_resulting_document` helper writes) — the restore handler's own
/// tiny reader for the shape that module owns as writer.
fn policy_write_from_resulting_document(doc: &Value) -> Result<PolicyWrite, String> {
    let enforced = doc
        .get("enforced")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let recommended = doc
        .get("recommended")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let parent_profile = doc
        .get("parent_profile")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let max_cache_age_secs = doc
        .get("max_cache_age_secs")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "resulting_document is missing max_cache_age_secs".to_string())?;
    let stale_action = match doc.get("stale_action").and_then(|v| v.as_str()) {
        Some("warn") => StaleAction::Warn,
        Some("refuse") => StaleAction::Refuse,
        _ => return Err("resulting_document has an invalid stale_action".to_string()),
    };
    Ok(PolicyWrite {
        enforced,
        recommended,
        parent_profile,
        max_cache_age_secs,
        stale_action,
    })
}

/// `POST /v1/admin/policy/{app}/{profile}/history/{version}/restore` — no
/// `If-Match` is accepted (spec 023 §7): there is nothing meaningful for a
/// client to have cached and compared against for "restore to version N,"
/// unlike an ordinary PUT/PATCH against the *current* document. The
/// `expected_version` this handler passes to
/// [`PolicyAdminStore::put_policy`] is simply whatever the profile's version
/// is right now, read immediately before writing — restore always targets
/// "make this the new latest version," not a specific prior version's
/// successor.
async fn restore_policy(
    State(state): State<Arc<ConfigServiceState>>,
    Path((app, profile, version)): Path<(String, String, u64)>,
    CallerClaims(claims): CallerClaims,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let history = match admin_store.policy_history(&app, &profile).await {
        Ok(h) => h,
        Err(e) => return internal_error("policy history lookup for restore", e),
    };
    let Some(entry) = history.iter().find(|e| {
        e.resulting_version == version
            && matches!(
                e.kind,
                MutationKind::PolicyPut | MutationKind::PolicyPatch | MutationKind::PolicyRestore
            )
    }) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let policy_write = match policy_write_from_resulting_document(&entry.resulting_document) {
        Ok(pw) => pw,
        Err(msg) => return internal_error("restore: could not decode resulting_document", msg),
    };

    let candidate = stored_policy_from_write(&app, &profile, &policy_write, 0);
    let errors = validate_policy_for_write(state.policy_store.as_ref(), &candidate).await;
    if !errors.is_empty() {
        return validation_error_response(errors);
    }

    let current_version = match state.policy_store.policy(&app, &profile).await {
        Ok(Some(p)) => p.version,
        Ok(None) => 0,
        Err(e) => return internal_error("policy lookup for restore expected_version", e),
    };

    let submitted = json!({ "restore_from_version": version });
    match admin_store
        .put_policy(
            &app,
            &profile,
            policy_write,
            MutationKind::PolicyRestore,
            submitted,
            &actor,
            current_version,
        )
        .await
    {
        Ok(v) => etag_response(v),
        Err(e) => admin_write_error_response(e),
    }
}

// ── Assignment rules: GET / PUT ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct AssignmentRuleWire {
    claim_path: String,
    operator: RuleOperator,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    profile: String,
}

impl From<&AssignmentRule> for AssignmentRuleWire {
    fn from(r: &AssignmentRule) -> Self {
        Self {
            claim_path: r.claim_path.clone(),
            operator: r.operator,
            value: r.value.clone(),
            profile: r.profile.clone(),
        }
    }
}

async fn get_assignments(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(_claims): CallerClaims,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let mut rules = match state.policy_store.assignment_rules(&app).await {
        Ok(r) => r,
        Err(e) => return internal_error("assignment rules lookup", e),
    };
    rules.sort_by_key(|r| r.ord);
    // Default rules are an internal representation detail (see
    // `RuleOperator::Default`'s docs) -- omitted from the admin-facing wire
    // shape the same way the bundle format folds them into `default_profile`
    // rather than an explicit rule. There is deliberately no wire
    // representation for "default profile" in this GET body; a caller wanting
    // one submits it as an explicit `Default`-operator... actually spec 023
    // does not require surfacing it distinctly, so it is simply included
    // like any other rule here, in its stored `ord` position.
    let version = match admin_store.assignment_rules_version(&app).await {
        Ok(v) => v,
        Err(e) => return internal_error("assignment rules version lookup", e),
    };
    let wire: Vec<AssignmentRuleWire> = rules.iter().map(AssignmentRuleWire::from).collect();
    let etag = format!("\"{version}\"");
    (
        StatusCode::OK,
        [(ETAG, etag)],
        Json(json!({ "rules": wire, "version": version })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct AssignmentRuleBody {
    claim_path: String,
    operator: String,
    #[serde(default)]
    value: Option<Value>,
    profile: String,
}

#[derive(Debug, Deserialize)]
struct PutAssignmentsBody {
    rules: Vec<AssignmentRuleBody>,
}

async fn put_assignments(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
    Json(raw): Json<Value>,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let expected_version = match require_if_match(&headers) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let body: PutAssignmentsBody = match serde_json::from_value(raw) {
        Ok(b) => b,
        Err(e) => {
            return bad_request(format!(
                "request body is not a valid assignment rule set: {e}"
            ))
        }
    };

    let mut rules = Vec::with_capacity(body.rules.len());
    for (idx, r) in body.rules.into_iter().enumerate() {
        let Some(operator) = RuleOperator::parse_wire_str(&r.operator) else {
            return bad_request(format!("rule {idx}: unknown operator '{}'", r.operator));
        };
        rules.push(AssignmentRule {
            app: app.clone(),
            // The server assigns `ord` from array position (spec 023) --
            // `idx` here, not anything the client could have sent (the
            // request body shape has no `ord` field at all).
            ord: idx as i64,
            claim_path: r.claim_path,
            operator,
            value: r.value,
            profile: r.profile,
        });
    }

    let existing_policies = match state.policy_store.policies_for_app(&app).await {
        Ok(p) => p,
        Err(e) => return internal_error("policies lookup for assignment validation", e),
    };
    let by_profile: HashMap<&str, &StoredPolicy> = existing_policies
        .iter()
        .map(|p| (p.profile.as_str(), p))
        .collect();
    let mut errors = assignment_rule_missing_profile_errors(&app, &rules, &by_profile);
    errors.extend(default_rule_not_last_errors(&app, &rules));
    if !errors.is_empty() {
        return validation_error_response(errors);
    }

    match admin_store
        .put_assignment_rules(&app, rules, &actor, expected_version)
        .await
    {
        Ok(v) => etag_response(v),
        Err(e) => admin_write_error_response(e),
    }
}

// ── Export / Import ──────────────────────────────────────────────────────────

async fn export_bundle(
    State(state): State<Arc<ConfigServiceState>>,
    CallerClaims(_claims): CallerClaims,
) -> Response {
    match build_export_tar(state.policy_store.as_ref()).await {
        Ok(bytes) => (StatusCode::OK, [(CONTENT_TYPE, "application/x-tar")], bytes).into_response(),
        Err(e) => internal_error("bundle export", e),
    }
}

async fn import_bundle_handler(
    State(state): State<Arc<ConfigServiceState>>,
    CallerClaims(claims): CallerClaims,
    body: Bytes,
) -> Response {
    let Some(admin_store) = state.admin_store.clone() else {
        return admin_store_not_configured();
    };
    let Some(actor) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let bundle: FsPolicyStore = match extract_bundle_from_tar(&body) {
        Ok(b) => b,
        Err(e) => return bad_request(format!("invalid bundle: {e}")),
    };

    match admin_store.import_bundle(&bundle, &actor).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(e) => admin_write_error_response(e),
    }
}
