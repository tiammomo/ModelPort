use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    config::{AppConfig, ResolvedProvider, RouteCandidateConfig, RoutingProfile, SmartRoutingMode},
    control::{ClientIdentity, ControlStore, ProviderRoutingSignal},
    error::AppError,
    exchange::ExchangeRequest,
    pricing::{self, TokenUsageBreakdown},
};

const INPUT_CHARS_PER_TOKEN: usize = 4;
const SESSION_AFFINITY_WEIGHT: f64 = 0.02;

#[derive(Debug, Default)]
pub(crate) struct SmartRouter {
    inner: Mutex<RouterState>,
}

#[derive(Debug, Default)]
struct RouterState {
    policy_version: String,
    decisions_total: u64,
    active_decisions_total: u64,
    shadow_decisions_total: u64,
    static_decisions_total: u64,
    shadow_disagreements_total: u64,
    selected_by_candidate: BTreeMap<String, u64>,
    recommended_by_candidate: BTreeMap<String, u64>,
    tracked_candidates: BTreeSet<String>,
    outcomes: BTreeMap<String, CandidateOutcome>,
}

#[derive(Debug, Clone, Copy, Default)]
struct CandidateOutcome {
    attempts: u64,
    successes: u64,
    latency_ewma_ms: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct RoutePlan {
    pub(crate) attempts: Vec<ResolvedProvider>,
    pub(crate) evidence: RoutingDecisionEvidence,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingDecisionEvidence {
    pub(crate) decision_id: String,
    pub(crate) group_id: Option<String>,
    pub(crate) profile: String,
    pub(crate) policy_version: String,
    pub(crate) mode: String,
    pub(crate) candidate_count: usize,
    pub(crate) selected_provider: String,
    pub(crate) selected_model: String,
    pub(crate) recommended_provider: String,
    pub(crate) recommended_model: String,
    pub(crate) selected_score: f64,
    pub(crate) recommended_score: f64,
    pub(crate) reason_codes: Vec<String>,
    pub(crate) session_affinity: bool,
    pub(crate) shadow_disagreement: bool,
}

pub(crate) struct RoutingRequest<'a> {
    pub(crate) config: &'a AppConfig,
    pub(crate) control: &'a ControlStore,
    pub(crate) identity: &'a ClientIdentity,
    pub(crate) client_ip: Option<&'a str>,
    pub(crate) exchange: &'a ExchangeRequest,
    pub(crate) profile_override: Option<RoutingProfile>,
    pub(crate) session_hash: Option<&'a str>,
    pub(crate) activation_key: &'a str,
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    resolved: ResolvedProvider,
    quality: f64,
    latency_hint_ms: u64,
    score: f64,
    original_index: usize,
}

impl SmartRouter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn plan(&self, request: RoutingRequest<'_>) -> Result<RoutePlan, AppError> {
        self.sync_policy_candidates(request.config);
        let Some((group_id, group)) = request
            .config
            .smart_route_group(&request.exchange.requested_model)
        else {
            return self.static_plan(&request);
        };
        let profile = request
            .profile_override
            .or(group.default_profile)
            .unwrap_or(request.config.smart_routing.default_profile);
        let control_signals = request.control.provider_routing_signals();
        let runtime_outcomes = self
            .inner
            .lock()
            .expect("smart router lock poisoned")
            .outcomes
            .clone();

        let mut rejection_counts = BTreeMap::<&'static str, usize>::new();
        let mut gate_error = None;
        let mut eligible = collect_eligible_candidates(
            &request,
            group.candidates.as_slice(),
            true,
            &mut rejection_counts,
            &mut gate_error,
        );
        let used_cooling_fallback = eligible.is_empty()
            && rejection_counts
                .get("provider_cooldown")
                .copied()
                .unwrap_or(0)
                > 0;
        if used_cooling_fallback {
            rejection_counts.clear();
            gate_error = None;
            eligible = collect_eligible_candidates(
                &request,
                group.candidates.as_slice(),
                false,
                &mut rejection_counts,
                &mut gate_error,
            );
        }
        if eligible.is_empty() {
            if let Some(error) = gate_error {
                return Err(error);
            }
            let credential_rejections = rejection_counts
                .get("credential")
                .copied()
                .unwrap_or_default();
            let disabled_candidates = rejection_counts
                .get("disabled")
                .copied()
                .unwrap_or_default();
            if credential_rejections > 0
                && credential_rejections.saturating_add(disabled_candidates)
                    == group.candidates.len()
            {
                return Err(AppError::NotReady(format!(
                    "smart route group `{group_id}` has no credential-ready candidate"
                )));
            }
            return Err(AppError::ProviderNotFound(format!(
                "smart route group `{group_id}` has no eligible candidate"
            )));
        }

        let input_tokens = request
            .exchange
            .serialized_input_chars()
            .div_ceil(INPUT_CHARS_PER_TOKEN)
            .try_into()
            .unwrap_or(u64::MAX);
        let output_tokens = request.exchange.estimated_output_tokens();
        let mut scored = score_candidates(
            eligible.clone(),
            profile,
            input_tokens,
            output_tokens,
            &control_signals,
            &runtime_outcomes,
            request.session_hash,
        );
        scored.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.original_index.cmp(&right.original_index))
        });

        let recommended = scored
            .first()
            .cloned()
            .expect("eligible candidates produce a recommendation");
        let canary_active = request.config.smart_routing.mode == SmartRoutingMode::Active
            && activation_bucket(request.activation_key)
                < request.config.smart_routing.activation_percent;
        let route_is_active =
            request.config.smart_routing.mode == SmartRoutingMode::Active && canary_active;
        let attempts = if route_is_active {
            scored
                .iter()
                .map(|candidate| candidate.resolved.clone())
                .collect::<Vec<_>>()
        } else {
            eligible
                .iter()
                .map(|candidate| candidate.resolved.clone())
                .collect::<Vec<_>>()
        };
        let selected = attempts
            .first()
            .expect("eligible route plan must select a candidate");
        let selected_score = scored
            .iter()
            .find(|candidate| {
                candidate.resolved.provider_id == selected.provider_id
                    && candidate.resolved.model == selected.model
            })
            .map(|candidate| candidate.score)
            .unwrap_or(0.0);
        let mode = match request.config.smart_routing.mode {
            SmartRoutingMode::Shadow => "shadow",
            SmartRoutingMode::Active if route_is_active => "active",
            SmartRoutingMode::Active => "canary_control",
            SmartRoutingMode::Off => "off_static",
        };
        let mut reason_codes = vec![
            format!("profile_{}", profile.as_str()),
            "quality_prior".to_owned(),
            "expected_cost".to_owned(),
            "latency_hint".to_owned(),
            "runtime_reliability".to_owned(),
        ];
        if request.session_hash.is_some() {
            reason_codes.push("session_affinity".to_owned());
        }
        if used_cooling_fallback {
            reason_codes.push("all_candidates_cooling".to_owned());
        } else if rejection_counts
            .get("provider_cooldown")
            .copied()
            .unwrap_or(0)
            > 0
        {
            reason_codes.push("provider_cooldown_filtered".to_owned());
        }
        if request.config.smart_routing.mode == SmartRoutingMode::Off {
            reason_codes.push("smart_routing_off".to_owned());
        } else if !route_is_active {
            reason_codes.push(
                if request.config.smart_routing.mode == SmartRoutingMode::Shadow {
                    "shadow_no_route_change"
                } else {
                    "canary_control_no_route_change"
                }
                .to_owned(),
            );
        }
        for reason in rejection_counts.keys().take(3) {
            reason_codes.push(format!("filtered_{reason}"));
        }
        let shadow_disagreement = matches!(mode, "shadow" | "canary_control")
            && (selected.provider_id != recommended.resolved.provider_id
                || selected.model != recommended.resolved.model);
        let evidence = RoutingDecisionEvidence {
            decision_id: new_decision_id(),
            group_id: Some(group_id.to_owned()),
            profile: profile.as_str().to_owned(),
            policy_version: request.config.smart_routing.policy_version.clone(),
            mode: mode.to_owned(),
            candidate_count: eligible.len(),
            selected_provider: selected.provider_id.clone(),
            selected_model: selected.model.clone(),
            recommended_provider: recommended.resolved.provider_id.clone(),
            recommended_model: recommended.resolved.model.clone(),
            selected_score: round_score(selected_score),
            recommended_score: round_score(recommended.score),
            reason_codes,
            session_affinity: request.session_hash.is_some(),
            shadow_disagreement,
        };
        self.record_decision(&evidence);
        Ok(RoutePlan { attempts, evidence })
    }

    fn static_plan(&self, request: &RoutingRequest<'_>) -> Result<RoutePlan, AppError> {
        let primary = request.config.resolve(&request.exchange.requested_model)?;
        let attempts = static_route_attempts(
            request.control,
            request.config,
            &request.exchange.requested_model,
            primary,
        );
        let selected = attempts
            .first()
            .expect("static route planning always returns a candidate");
        let reason_codes =
            if is_explicit_provider_selector(request.config, &request.exchange.requested_model) {
                vec!["explicit_provider_model".to_owned()]
            } else {
                vec![
                    "deterministic_resolution".to_owned(),
                    "configured_provider_order".to_owned(),
                ]
            };
        let evidence = RoutingDecisionEvidence {
            decision_id: new_decision_id(),
            group_id: None,
            profile: "explicit".to_owned(),
            policy_version: "static-v1".to_owned(),
            mode: "static".to_owned(),
            candidate_count: attempts.len(),
            selected_provider: selected.provider_id.clone(),
            selected_model: selected.model.clone(),
            recommended_provider: selected.provider_id.clone(),
            recommended_model: selected.model.clone(),
            selected_score: 1.0,
            recommended_score: 1.0,
            reason_codes,
            session_affinity: false,
            shadow_disagreement: false,
        };
        self.record_decision(&evidence);
        Ok(RoutePlan { attempts, evidence })
    }

    pub(crate) fn record_outcome(
        &self,
        provider_id: &str,
        model: &str,
        success: bool,
        latency: Duration,
    ) {
        let key = candidate_key(provider_id, model);
        let mut inner = self.inner.lock().expect("smart router lock poisoned");
        if !inner.tracked_candidates.contains(&key) {
            return;
        }
        let outcome = inner.outcomes.entry(key).or_default();
        outcome.attempts = outcome.attempts.saturating_add(1);
        outcome.successes = outcome.successes.saturating_add(u64::from(success));
        let latency_ms = latency.as_secs_f64() * 1_000.0;
        outcome.latency_ewma_ms = if outcome.attempts == 1 {
            latency_ms
        } else {
            outcome.latency_ewma_ms * 0.8 + latency_ms * 0.2
        };
    }

    pub(crate) fn status(&self, config: &AppConfig) -> Value {
        self.sync_policy_candidates(config);
        let inner = self.inner.lock().expect("smart router lock poisoned");
        let groups = config
            .smart_routing
            .groups
            .iter()
            .map(|(id, group)| {
                json!({
                    "id": id,
                    "aliases": group.aliases,
                    "defaultProfile": group.default_profile
                        .unwrap_or(config.smart_routing.default_profile)
                        .as_str(),
                    "candidateCount": group.candidates.iter().filter(|candidate| candidate.enabled).count(),
                })
            })
            .collect::<Vec<_>>();
        let outcomes = inner
            .outcomes
            .iter()
            .map(|(candidate, outcome)| {
                json!({
                    "candidate": candidate,
                    "attempts": outcome.attempts,
                    "successes": outcome.successes,
                    "successRate": ratio(outcome.successes, outcome.attempts),
                    "latencyEwmaMs": outcome.latency_ewma_ms.round() as u64,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "mode": config.smart_routing.mode.as_str(),
            "defaultProfile": config.smart_routing.default_profile.as_str(),
            "policyVersion": config.smart_routing.policy_version,
            "activationPercent": config.smart_routing.activation_percent,
            "groups": groups,
            "decisionsTotal": inner.decisions_total,
            "activeDecisionsTotal": inner.active_decisions_total,
            "shadowDecisionsTotal": inner.shadow_decisions_total,
            "staticDecisionsTotal": inner.static_decisions_total,
            "shadowDisagreementsTotal": inner.shadow_disagreements_total,
            "selectedByCandidate": inner.selected_by_candidate,
            "recommendedByCandidate": inner.recommended_by_candidate,
            "outcomes": outcomes,
        })
    }

    fn record_decision(&self, decision: &RoutingDecisionEvidence) {
        let mut inner = self.inner.lock().expect("smart router lock poisoned");
        inner.decisions_total = inner.decisions_total.saturating_add(1);
        if decision.mode == "active" {
            inner.active_decisions_total = inner.active_decisions_total.saturating_add(1);
        } else if matches!(decision.mode.as_str(), "shadow" | "canary_control") {
            inner.shadow_decisions_total = inner.shadow_decisions_total.saturating_add(1);
        } else {
            inner.static_decisions_total = inner.static_decisions_total.saturating_add(1);
        }
        if decision.shadow_disagreement
            && matches!(decision.mode.as_str(), "shadow" | "canary_control")
        {
            inner.shadow_disagreements_total = inner.shadow_disagreements_total.saturating_add(1);
        }
        if decision.group_id.is_some() {
            let key = candidate_key(&decision.selected_provider, &decision.selected_model);
            let selected = inner.selected_by_candidate.entry(key).or_default();
            *selected = selected.saturating_add(1);
            let key = candidate_key(&decision.recommended_provider, &decision.recommended_model);
            let recommended = inner.recommended_by_candidate.entry(key).or_default();
            *recommended = recommended.saturating_add(1);
        }
    }

    fn sync_policy_candidates(&self, config: &AppConfig) {
        let configured_candidates = config
            .smart_routing
            .groups
            .values()
            .flat_map(|group| group.candidates.iter())
            .filter(|candidate| candidate.enabled)
            .map(|candidate| candidate_key(&candidate.provider, &candidate.model))
            .collect::<BTreeSet<_>>();
        let mut inner = self.inner.lock().expect("smart router lock poisoned");
        if inner.policy_version != config.smart_routing.policy_version
            || inner.tracked_candidates != configured_candidates
        {
            *inner = RouterState {
                policy_version: config.smart_routing.policy_version.clone(),
                tracked_candidates: configured_candidates,
                ..RouterState::default()
            };
        }
    }
}

fn collect_eligible_candidates(
    request: &RoutingRequest<'_>,
    candidates: &[RouteCandidateConfig],
    filter_cooldown: bool,
    rejection_counts: &mut BTreeMap<&'static str, usize>,
    gate_error: &mut Option<AppError>,
) -> Vec<ScoredCandidate> {
    let mut eligible = Vec::new();
    for (original_index, candidate) in candidates.iter().enumerate() {
        match candidate_rejection(request, candidate, filter_cooldown) {
            Ok(Some(rejection)) => {
                let count = rejection_counts.entry(rejection).or_default();
                *count = count.saturating_add(1);
                continue;
            }
            Err(error) => {
                let count = rejection_counts.entry("policy").or_default();
                *count = count.saturating_add(1);
                if gate_error.is_none() {
                    *gate_error = Some(error);
                }
                continue;
            }
            Ok(None) => {}
        }
        let provider = request
            .config
            .providers
            .get(&candidate.provider)
            .expect("validated routing candidate provider")
            .clone();
        eligible.push(ScoredCandidate {
            resolved: ResolvedProvider {
                provider_id: candidate.provider.clone(),
                provider,
                model: candidate.model.clone(),
            },
            quality: candidate.quality,
            latency_hint_ms: candidate.latency_hint_ms,
            score: 0.0,
            original_index,
        });
    }
    eligible
}

fn candidate_rejection(
    request: &RoutingRequest<'_>,
    candidate: &RouteCandidateConfig,
    filter_cooldown: bool,
) -> Result<Option<&'static str>, AppError> {
    if !candidate.enabled {
        return Ok(Some("disabled"));
    }
    let Some(provider) = request.config.providers.get(&candidate.provider) else {
        return Ok(Some("missing_provider"));
    };
    match request
        .control
        .provider_credential_route_available(&candidate.provider)
    {
        Some(false) => return Ok(Some("credential")),
        None if provider.api_key_required && provider.api_key().ok().flatten().is_none() => {
            return Ok(Some("credential"));
        }
        Some(true) | None => {}
    }
    if filter_cooldown && request.control.provider_in_cooldown(&candidate.provider) {
        return Ok(Some("provider_cooldown"));
    }
    let resolved = ResolvedProvider {
        provider_id: candidate.provider.clone(),
        provider: provider.clone(),
        model: candidate.model.clone(),
    };
    if request.exchange.validate_provider(&resolved).is_err() {
        return Ok(Some("capability"));
    }
    request.control.check_quotas(
        request.identity,
        request.client_ip,
        &request.exchange.requested_model,
        &candidate.model,
        &candidate.provider,
    )?;
    Ok(None)
}

fn score_candidates(
    mut candidates: Vec<ScoredCandidate>,
    profile: RoutingProfile,
    input_tokens: u64,
    output_tokens: u64,
    control_signals: &BTreeMap<String, ProviderRoutingSignal>,
    runtime_outcomes: &BTreeMap<String, CandidateOutcome>,
    session_hash: Option<&str>,
) -> Vec<ScoredCandidate> {
    let raw = candidates
        .iter()
        .map(|candidate| {
            let outcome = runtime_outcomes.get(&candidate_key(
                &candidate.resolved.provider_id,
                &candidate.resolved.model,
            ));
            let control = control_signals.get(&candidate.resolved.provider_id);
            let attempts = control.map_or(0, |value| value.requests_total);
            let successes = control.map_or(0, |value| value.successes_total);
            let reliability = (successes as f64 + 2.0) / (attempts as f64 + 4.0);
            let latency = outcome.filter(|value| value.attempts > 0).map_or(
                candidate.latency_hint_ms as f64,
                |value| {
                    blend_latency_hint(
                        candidate.latency_hint_ms as f64,
                        value.latency_ewma_ms,
                        value.attempts,
                    )
                },
            );
            let usage = TokenUsageBreakdown {
                input_tokens,
                output_tokens,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            };
            let cost = pricing::cost_for_model_with_pricing(
                &candidate.resolved.model,
                usage,
                candidate.resolved.provider.pricing,
            );
            (candidate.quality, reliability, latency, cost)
        })
        .collect::<Vec<_>>();
    let latency_min = raw
        .iter()
        .map(|value| value.2)
        .fold(f64::INFINITY, f64::min);
    let latency_max = raw.iter().map(|value| value.2).fold(0.0, f64::max);
    let cost_min = raw
        .iter()
        .map(|value| value.3)
        .fold(f64::INFINITY, f64::min);
    let cost_max = raw.iter().map(|value| value.3).fold(0.0, f64::max);
    let weights = profile_weights(profile);

    for (index, candidate) in candidates.iter_mut().enumerate() {
        let (quality, reliability, latency, cost) = raw[index];
        let latency_value = inverse_normalized(latency, latency_min, latency_max);
        let cost_value = inverse_normalized(cost, cost_min, cost_max);
        let affinity = session_hash.map_or(0.0, |session_hash| {
            affinity_value(
                session_hash,
                &candidate.resolved.provider_id,
                &candidate.resolved.model,
            ) * SESSION_AFFINITY_WEIGHT
        });
        candidate.score = quality * weights.quality
            + reliability * weights.reliability
            + latency_value * weights.latency
            + cost_value * weights.cost
            + affinity;
    }
    candidates
}

#[derive(Debug, Clone, Copy)]
struct ProfileWeights {
    quality: f64,
    cost: f64,
    latency: f64,
    reliability: f64,
}

fn profile_weights(profile: RoutingProfile) -> ProfileWeights {
    match profile {
        RoutingProfile::Quality => ProfileWeights {
            quality: 0.60,
            cost: 0.10,
            latency: 0.15,
            reliability: 0.15,
        },
        RoutingProfile::Balanced => ProfileWeights {
            quality: 0.35,
            cost: 0.30,
            latency: 0.20,
            reliability: 0.15,
        },
        RoutingProfile::Economy => ProfileWeights {
            quality: 0.15,
            cost: 0.70,
            latency: 0.05,
            reliability: 0.10,
        },
        RoutingProfile::Latency => ProfileWeights {
            quality: 0.20,
            cost: 0.10,
            latency: 0.60,
            reliability: 0.10,
        },
    }
}

fn inverse_normalized(value: f64, min: f64, max: f64) -> f64 {
    if !value.is_finite()
        || !min.is_finite()
        || !max.is_finite()
        || (max - min).abs() < f64::EPSILON
    {
        return 1.0;
    }
    1.0 - ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn blend_latency_hint(hint_ms: f64, observed_ms: f64, attempts: u64) -> f64 {
    let samples = attempts.min(1_000) as f64;
    let confidence = samples / (samples + 10.0);
    hint_ms * (1.0 - confidence) + observed_ms * confidence
}

fn affinity_value(session_hash: &str, provider_id: &str, model: &str) -> f64 {
    let digest = Sha256::digest(format!("{session_hash}\0{provider_id}\0{model}").as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes) as f64 / u64::MAX as f64
}

fn activation_bucket(key: &str) -> u8 {
    let digest = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u8::try_from(u64::from_be_bytes(bytes) % 100).expect("modulo 100 fits in u8")
}

fn candidate_key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}:{model}")
}

fn new_decision_id() -> String {
    format!("rtd_{}", Uuid::new_v4().simple())
}

pub(crate) fn hash_session_key(principal_id: &str, session_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(principal_id.as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn static_route_attempts(
    control: &ControlStore,
    config: &AppConfig,
    requested_model: &str,
    primary: ResolvedProvider,
) -> Vec<ResolvedProvider> {
    if is_explicit_provider_selector(config, requested_model) {
        return vec![primary];
    }
    let mut attempts = Vec::new();
    if !control.provider_in_cooldown(&primary.provider_id) {
        attempts.push(primary.clone());
    }

    for provider_id in &config.provider_order {
        if provider_id == &primary.provider_id || control.provider_in_cooldown(provider_id) {
            continue;
        }
        let Some(provider) = config.providers.get(provider_id) else {
            continue;
        };
        let Some(model) = fallback_model_for_provider(provider, requested_model, &primary.model)
        else {
            continue;
        };
        attempts.push(ResolvedProvider {
            provider_id: provider_id.clone(),
            provider: provider.clone(),
            model,
        });
    }

    if attempts.is_empty() {
        attempts.push(primary);
    }
    attempts
}

fn is_explicit_provider_selector(config: &AppConfig, requested_model: &str) -> bool {
    requested_model
        .split_once(':')
        .is_some_and(|(provider_id, model)| {
            !model.is_empty() && config.providers.contains_key(provider_id)
        })
}

fn fallback_model_for_provider(
    provider: &crate::config::ProviderConfig,
    requested_model: &str,
    primary_model: &str,
) -> Option<String> {
    for model in [requested_model, primary_model] {
        if provider.models.iter().any(|configured| configured == model)
            || provider
                .model_prefixes
                .iter()
                .any(|prefix| model.starts_with(prefix))
            || provider.passthrough_unknown_models
        {
            return Some(model.to_owned());
        }
    }
    None
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        (numerator as f64 / denominator as f64 * 10_000.0).round() / 100.0
    }
}

fn round_score(score: f64) -> f64 {
    (score * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::SocketAddr};

    use serde_json::{Map, json};

    use super::*;
    use crate::{
        config::{
            FidelityMode, MaxTokensField, ProviderConfig, ProviderProtocol, ReasoningConfig,
            RouteCandidateConfig, RouteGroupConfig, SamplingConfig, SmartRoutingConfig,
            TokenCountingConfig, ToolUseConfig,
        },
        control::ControlStore,
        exchange::{ClientRequest, OpenAiChatRequest},
        pricing::ModelPricing,
    };

    fn provider(model: &str, quality_tier: bool) -> ProviderConfig {
        ProviderConfig {
            display_name: model.to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url: "https://provider.example/v1".to_owned(),
            api_key_env: None,
            api_key: Some("test-only".to_owned()),
            api_key_required: true,
            default_model: model.to_owned(),
            models: vec![model.to_owned()],
            model_prefixes: Vec::new(),
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: ToolUseConfig::default(),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: Some(ModelPricing {
                input_per_million: if quality_tier { 30.0 } else { 0.1 },
                output_per_million: if quality_tier { 60.0 } else { 0.2 },
                cache_write_per_million: 0.0,
                cache_read_per_million: 0.0,
            }),
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        }
    }

    fn config(mode: SmartRoutingMode, activation_percent: u8) -> AppConfig {
        let premium = provider("premium-model", true);
        let economy = provider("economy-model", false);
        AppConfig {
            bind_addr: "127.0.0.1:17878".parse::<SocketAddr>().unwrap(),
            max_request_body_bytes: 1024 * 1024,
            max_concurrent_requests: 64,
            auth_token: None,
            default_provider: "premium".to_owned(),
            provider_order: vec!["premium".to_owned(), "economy".to_owned()],
            providers: HashMap::from([
                ("premium".to_owned(), premium),
                ("economy".to_owned(), economy),
            ]),
            aliases: HashMap::new(),
            smart_routing: SmartRoutingConfig {
                mode,
                default_profile: RoutingProfile::Balanced,
                policy_version: "test-v1".to_owned(),
                activation_percent,
                groups: HashMap::from([(
                    "general".to_owned(),
                    RouteGroupConfig {
                        aliases: vec!["auto".to_owned()],
                        default_profile: None,
                        candidates: vec![
                            RouteCandidateConfig {
                                provider: "premium".to_owned(),
                                model: "premium-model".to_owned(),
                                quality: 0.99,
                                latency_hint_ms: 800,
                                enabled: true,
                            },
                            RouteCandidateConfig {
                                provider: "economy".to_owned(),
                                model: "economy-model".to_owned(),
                                quality: 0.50,
                                latency_hint_ms: 800,
                                enabled: true,
                            },
                        ],
                    },
                )]),
            },
            runtime_adapters: Default::default(),
        }
    }

    fn exchange(model: &str) -> ExchangeRequest {
        ExchangeRequest::from_client(ClientRequest::OpenAiChat(OpenAiChatRequest {
            model: model.to_owned(),
            messages: vec![json!({"role": "user", "content": "route this"})],
            max_completion_tokens: Some(512),
            max_tokens: None,
            stream: Some(false),
            extra: Map::new(),
        }))
        .unwrap()
    }

    fn plan(
        router: &SmartRouter,
        config: &AppConfig,
        exchange: &ExchangeRequest,
        profile: Option<RoutingProfile>,
        activation_key: &str,
    ) -> Result<RoutePlan, AppError> {
        let control = ControlStore::for_tests();
        let identity = ControlStore::legacy_identity();
        router.plan(RoutingRequest {
            config,
            control: &control,
            identity: &identity,
            client_ip: Some("127.0.0.1"),
            exchange,
            profile_override: profile,
            session_hash: None,
            activation_key,
        })
    }

    #[test]
    fn lower_cost_wins_the_economy_profile() {
        assert!(
            profile_weights(RoutingProfile::Economy).cost
                > profile_weights(RoutingProfile::Economy).quality
        );
    }

    #[test]
    fn affinity_and_activation_are_stable() {
        assert_eq!(
            affinity_value("session", "provider", "model"),
            affinity_value("session", "provider", "model")
        );
        assert_eq!(activation_bucket("request"), activation_bucket("request"));
    }

    #[test]
    fn normalization_rewards_lower_values() {
        assert!(inverse_normalized(10.0, 10.0, 100.0) > inverse_normalized(100.0, 10.0, 100.0));
    }

    #[test]
    fn latency_observations_gain_weight_conservatively() {
        let one_sample = blend_latency_hint(1_000.0, 100.0, 1);
        let many_samples = blend_latency_hint(1_000.0, 100.0, 100);
        assert!(one_sample > many_samples);
        assert!(one_sample > 900.0);
        assert!(many_samples < 200.0);
    }

    #[test]
    fn active_profiles_choose_cost_or_quality_without_changing_the_alias_contract() {
        let router = SmartRouter::new();
        let config = config(SmartRoutingMode::Active, 100);
        let exchange = exchange("auto");

        let economy = plan(
            &router,
            &config,
            &exchange,
            Some(RoutingProfile::Economy),
            "economy-request",
        )
        .unwrap();
        assert_eq!(economy.attempts[0].provider_id, "economy");
        assert_eq!(economy.evidence.mode, "active");
        assert_eq!(economy.evidence.profile, "economy");

        let quality = plan(
            &router,
            &config,
            &exchange,
            Some(RoutingProfile::Quality),
            "quality-request",
        )
        .unwrap();
        assert_eq!(quality.attempts[0].provider_id, "premium");
        assert_eq!(quality.evidence.profile, "quality");
    }

    #[test]
    fn shadow_mode_preserves_configured_order_and_records_disagreement() {
        let router = SmartRouter::new();
        let config = config(SmartRoutingMode::Shadow, 100);
        let exchange = exchange("auto");

        let planned = plan(
            &router,
            &config,
            &exchange,
            Some(RoutingProfile::Economy),
            "shadow-request",
        )
        .unwrap();

        assert_eq!(planned.attempts[0].provider_id, "premium");
        assert_eq!(planned.evidence.recommended_provider, "economy");
        assert!(planned.evidence.shadow_disagreement);
        assert!(
            planned
                .evidence
                .reason_codes
                .contains(&"shadow_no_route_change".to_owned())
        );
    }

    #[test]
    fn zero_percent_canary_and_off_mode_keep_the_configured_baseline() {
        let router = SmartRouter::new();
        let canary = config(SmartRoutingMode::Active, 0);
        let exchange = exchange("auto");
        let planned = plan(
            &router,
            &canary,
            &exchange,
            Some(RoutingProfile::Economy),
            "control-request",
        )
        .unwrap();
        assert_eq!(planned.attempts[0].provider_id, "premium");
        assert_eq!(planned.evidence.mode, "canary_control");
        assert_eq!(router.status(&canary)["shadowDisagreementsTotal"], 1);

        let disabled = config(SmartRoutingMode::Off, 100);
        let planned = plan(
            &router,
            &disabled,
            &exchange,
            Some(RoutingProfile::Economy),
            "disabled-request",
        )
        .unwrap();
        assert_eq!(planned.attempts[0].provider_id, "premium");
        assert_eq!(planned.evidence.mode, "off_static");
        assert_eq!(router.status(&disabled)["shadowDisagreementsTotal"], 1);
    }

    #[test]
    fn session_keys_are_stable_but_principal_scoped() {
        assert_eq!(
            hash_session_key("user-a", "session"),
            hash_session_key("user-a", "session")
        );
        assert_ne!(
            hash_session_key("user-a", "session"),
            hash_session_key("user-b", "session")
        );
    }

    #[test]
    fn explicit_provider_model_never_crosses_to_a_fallback_provider() {
        let router = SmartRouter::new();
        let mut config = config(SmartRoutingMode::Shadow, 0);
        config
            .providers
            .get_mut("economy")
            .unwrap()
            .models
            .push("premium-model".to_owned());

        let planned = plan(
            &router,
            &config,
            &exchange("premium:premium-model"),
            Some(RoutingProfile::Economy),
            "explicit-request",
        )
        .unwrap();

        assert_eq!(planned.attempts.len(), 1);
        assert_eq!(planned.attempts[0].provider_id, "premium");
        assert_eq!(
            planned.evidence.reason_codes,
            vec!["explicit_provider_model"]
        );
    }

    #[test]
    fn runtime_outcomes_track_only_configured_smart_candidates() {
        let router = SmartRouter::new();
        let config = config(SmartRoutingMode::Shadow, 0);
        router.record_outcome(
            "arbitrary",
            "user-controlled-model",
            true,
            Duration::from_millis(10),
        );
        assert_eq!(router.status(&config)["outcomes"], json!([]));

        plan(&router, &config, &exchange("auto"), None, "tracked-request").unwrap();
        router.record_outcome("premium", "premium-model", true, Duration::from_millis(10));
        assert_eq!(
            router.status(&config)["outcomes"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn smart_route_fails_ready_check_when_every_enabled_candidate_lacks_credentials() {
        let router = SmartRouter::new();
        let mut config = config(SmartRoutingMode::Shadow, 0);
        for provider in config.providers.values_mut() {
            provider.api_key = None;
        }

        let error = plan(
            &router,
            &config,
            &exchange("auto"),
            None,
            "credential-check",
        )
        .unwrap_err();
        assert!(matches!(error, AppError::NotReady(_)));
    }
}
