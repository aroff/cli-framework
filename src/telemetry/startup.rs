//! The fixed order in which telemetry starts up, and the code that walks it.
//!
//! Every entry is later than the one above for a specific reason, recorded on
//! the variant. Naming the order and pinning it with a test is cheap; finding
//! out that the boundary saw a policy mid-mutation is not.
//!
//! [`run_startup`] is the only thing that walks it. It is a pure function of
//! [`StartupInputs`] — no `std::env`, no `dirs::config_dir()`, no ambient
//! subscriber lookup that a caller cannot substitute — so the ordering can be
//! asserted against by running it, not by re-reading the constant it is
//! supposed to follow. [`run_startup_recording`] returns the steps startup
//! actually performed, which is the only honest oracle for an ordering: an
//! implementation that imports [`startup_order`] and then does something else
//! compiles fine and passes any test that only checks the constant.

// `DoctorFinding` lives at `crate::doctor`, not re-exported into `telemetry`
// (confirmed by grep: `subscriber.rs`'s own `foreign_subscriber_finding`
// reaches it the same full-path way), so it is named directly here rather
// than via `super::`.
use super::{
    Attribution, Deployment, KillSwitch, ServiceIdentity, StoreState, SubscriberOutcome, Surface,
    Telemetry, TelemetryGuard, TelemetryInputs, TelemetryLevel, TelemetryPolicy, TelemetryStore,
    TelemetryStoreLocation,
};
use crate::config::manifest::ConfigManifest;
use crate::config::resolution::Layer;
use crate::doctor::DoctorFinding;
use std::sync::Arc;

/// One step of telemetry startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStep {
    /// Before any disk or socket access — `DO_NOT_TRACK=1` must cost nothing.
    KillSwitches,
    /// Needed by resolution; failure is a value, never an abort.
    OpenStore,
    /// The generated section must exist before resolution can name its leaves.
    MergeManifest,
    /// One pure resolution, once per process.
    Resolve,
    /// Frozen into an `Arc` and never mutated after this point.
    FreezePolicy,
    /// Providers and the export boundary; they hold the policy by `Arc`.
    BuildProviders,
    /// After the providers, because the OTel layer needs a live tracer.
    InstallSubscriber,
    /// After the subscriber so it can be logged; before dispatch so a person
    /// sees it before the command's own output.
    ShowNotice,
    /// Last of the setup: a panic during startup should be reported by
    /// whatever hook was already installed, not by a half-built one.
    InstallPanicHook,
    /// The command itself.
    Dispatch,
}

/// The order, as a value so it can be asserted against.
pub fn startup_order() -> &'static [StartupStep] {
    &[
        StartupStep::KillSwitches,
        StartupStep::OpenStore,
        StartupStep::MergeManifest,
        StartupStep::Resolve,
        StartupStep::FreezePolicy,
        StartupStep::BuildProviders,
        StartupStep::InstallSubscriber,
        StartupStep::ShowNotice,
        StartupStep::InstallPanicHook,
        StartupStep::Dispatch,
    ]
}

/// What startup observed, as opposed to what it decided. The decisions live
/// in `TelemetryPolicy`; this is the evidence a person needs when the
/// decisions are not the ones they expected.
///
/// `Default` matters: the doctor tests construct one with a single field set
/// and `..Default::default()` for the rest, so a field added here does not
/// touch ten tests. That requires `SubscriberOutcome: Default` too, with
/// `Installed` as the default — the ordinary case.
#[derive(Debug, Clone, Default)]
pub struct StartupReport {
    pub subscriber: SubscriberOutcome,
    pub store: StoreState,
    pub kill_switch: Option<KillSwitch>,
    /// `<APP>_TELEMETRY_*` variables that matched no manifest leaf (PR2
    /// Task 7). Names only — never values, which is the whole point of
    /// reporting them at all.
    pub unmatched_env: Vec<String>,
    pub findings: Vec<DoctorFinding>,
}

/// A handle to a value that only telemetry startup can produce, handed out
/// before it exists.
///
/// The six telemetry doctor checks are registered at *build* time — that is
/// the only moment [`AppBuilder::push_doctor_checks`](crate::app::AppBuilder)
/// can reach the check list, because `build()` consumes the builder and moves
/// the list into the `doctor` command. Both values those checks read, the
/// `Arc<TelemetryPolicy>` and the `Arc<StartupReport>`, are produced at *run*
/// time by [`run_startup`]. A check handed the build-time policy would answer
/// about a resolution that had not opened the settings store: it would report
/// the level and attribution the app would have used had the person never run
/// `telemetry set`, which is exactly the question nobody is asking when they
/// run `doctor`.
///
/// So the checks hold this, filled at [`StartupStep::FreezePolicy`] and read
/// at the moment the doctor runs them.
///
/// Deliberately not a `OnceLock`. The policy cell is *seeded* at build time,
/// so an app that is built and never run — most of this crate's own suite,
/// and any app whose `doctor` command somehow runs before startup — still
/// gets an answer rather than a skipped check; startup then overwrites it
/// with the resolution that saw the store. `OnceLock` would make the seeding
/// and the real value mutually exclusive, and the seed would win.
///
/// [`get`](Self::get) clones the `Arc` out and drops the lock before it
/// returns, which is load-bearing rather than stylistic:
/// [`DoctorCheck::run`](crate::doctor::check::DoctorCheck::run) returns a
/// `'static` [`DoctorFuture`](crate::doctor::check::DoctorFuture), and a lock
/// guard alive across an `.await` is `clippy::await_holding_lock` — denied in
/// this workspace.
pub struct StartupCell<T> {
    slot: Arc<std::sync::RwLock<Option<Arc<T>>>>,
}

impl<T> StartupCell<T> {
    /// A cell nothing has filled yet. [`get`](Self::get) answers `None` until
    /// [`set`](Self::set) is called.
    pub fn empty() -> Self {
        Self {
            slot: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// A cell that already holds `value`.
    pub fn filled(value: Arc<T>) -> Self {
        Self {
            slot: Arc::new(std::sync::RwLock::new(Some(value))),
        }
    }

    /// Replace whatever the cell holds. Every clone of this cell sees it.
    ///
    /// Overwrites rather than refusing: the policy cell is seeded at build
    /// time and startup's resolution is the one that folded in the stored
    /// consent, so the later write is the one that must win.
    pub fn set(&self, value: Arc<T>) {
        let mut slot = self
            .slot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(value);
    }

    /// The value, if the cell has been filled.
    ///
    /// Clones the `Arc` out and releases the lock before returning; see the
    /// type's documentation for why that is a requirement and not a taste.
    ///
    /// A poisoned lock is read through rather than panicked on. The doctor
    /// exists to explain a broken process; making it the one command that
    /// panics inside a broken process would be backwards.
    pub fn get(&self) -> Option<Arc<T>> {
        self.slot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Manual, because `#[derive(Clone)]` would demand `T: Clone` — and `T` here
/// is `TelemetryPolicy` / `StartupReport` behind an `Arc`, which is precisely
/// the thing being shared rather than cloned. Cloning a cell shares the slot:
/// that is how the copy `App` keeps and the copies the six checks hold are
/// the same cell.
impl<T> Clone for StartupCell<T> {
    fn clone(&self) -> Self {
        Self {
            slot: Arc::clone(&self.slot),
        }
    }
}

impl<T> Default for StartupCell<T> {
    fn default() -> Self {
        Self::empty()
    }
}

/// So a caller that already has the value — every test that builds a policy
/// and a report by hand — passes it straight to
/// [`telemetry_checks`](crate::telemetry::telemetry_checks) with no ceremony.
impl<T> From<Arc<T>> for StartupCell<T> {
    fn from(value: Arc<T>) -> Self {
        Self::filled(value)
    }
}

impl<T> std::fmt::Debug for StartupCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The contents are deliberately not printed: `TelemetryPolicy` is
        // reachable from `App`'s `Debug`, and it carries the install id.
        f.debug_struct("StartupCell")
            .field("filled", &self.get().is_some())
            .finish()
    }
}

/// Everything one process needs in order to start telemetry, gathered by the
/// caller before the first step runs.
///
/// The plan sketched `run_startup(builder: &AppBuilder)`. That signature
/// cannot serve the caller that matters: [`crate::App::run_with_args`] holds
/// an `App`, not an `AppBuilder` — `build()` consumed the builder long before
/// the first command runs — so a `&AppBuilder` parameter would force startup
/// to happen at *build* time, and every `build(ctx)` call in a test suite
/// would open the real framework store in the developer's own configuration
/// directory. `AppBuilder` assembles this value instead,
/// `App::startup_inputs` hands it over, and startup stays a pure function of
/// it.
///
/// `env` is a *snapshot*, never `std::env` read from inside: one read at a
/// known point is substitutable in a test and cannot race a `set_var` on
/// another thread.
#[derive(Debug, Clone)]
pub struct StartupInputs {
    /// The layers the caller already knows: defaults, flags and builder
    /// overrides. Startup folds the stored and environment layers into this.
    pub base: TelemetryInputs,
    /// Where the framework-owned settings file lives, unresolved. Startup
    /// opens it at [`StartupStep::OpenStore`] — and not at all under a kill
    /// switch, which is the point of taking a location rather than a store.
    pub store: TelemetryStoreLocation,
    /// The application's manifest with the generated `telemetry` section
    /// already merged in. The environment layer needs its leaves to know
    /// which `<APP>_TELEMETRY_*` variables are real and which are typos.
    pub manifest: Arc<ConfigManifest>,
    pub service: ServiceIdentity,
    pub surface: Surface,
    pub stderr_is_tty: bool,
    /// `(name, value)` pairs, in the caller's order.
    pub env: Vec<(String, String)>,
}

/// What startup produced.
///
/// No `Debug`: `dyn Telemetry` has none, and a hand-written impl would only
/// restate fields a caller can already read.
pub struct StartupResult {
    /// Frozen at [`StartupStep::FreezePolicy`] and never mutated after —
    /// every consumer holds this same `Arc`.
    pub policy: Arc<TelemetryPolicy>,
    pub report: Arc<StartupReport>,
    /// `None` whenever nothing exports: a kill switch, `off`, or no endpoint.
    pub handle: Option<Arc<dyn Telemetry + Send + Sync>>,
    pub guard: Option<TelemetryGuard>,
    /// The first-run notice, for the caller to print. Startup does not write
    /// to stderr here: the caller owns the process's output and the ordering
    /// against its own banner.
    pub notice: Option<String>,
    /// Whether [`StartupStep::InstallPanicHook`] installed one. It does so
    /// only when something exports — a hook with nowhere to send is a hook
    /// that only slows a crash down.
    pub panic_hook: bool,
}

/// Start telemetry, in the order [`startup_order`] names.
pub fn run_startup(inputs: StartupInputs) -> StartupResult {
    run_startup_inner(inputs, &mut |_| {})
}

/// [`run_startup`], plus the steps it actually performed.
///
/// `run_startup` is implemented in terms of this one, passing a recorder that
/// discards. Two copies of an ordering are two orderings.
#[doc(hidden)]
pub fn run_startup_recording(inputs: StartupInputs) -> (StartupResult, Vec<StartupStep>) {
    let mut recorded = Vec::new();
    let result = run_startup_inner(inputs, &mut |step| recorded.push(step));
    (result, recorded)
}

fn run_startup_inner(inputs: StartupInputs, record: &mut dyn FnMut(StartupStep)) -> StartupResult {
    let StartupInputs {
        mut base,
        store: location,
        manifest,
        service,
        surface,
        stderr_is_tty,
        env,
    } = inputs;
    let mut report = StartupReport::default();

    // 1. Kill switches, before any disk or socket access. Someone who set
    //    `DO_NOT_TRACK=1` should not pay for a `create_dir_all`.
    record(StartupStep::KillSwitches);
    let kill_switch = detect_kill_switch_in(&base.app, &env).or(base.kill_switch);
    base.kill_switch = kill_switch;
    report.kill_switch = kill_switch;

    // 2 and 3. The store and the environment. Both are skipped outright under
    //    a kill switch: there is nothing left for them to decide, and opening
    //    the store would create a directory on the disk of a person who just
    //    asked for none of this.
    let mut store = None;
    let mut notice_shown = None;
    if kill_switch.is_none() {
        record(StartupStep::OpenStore);
        let opened = location.open(&base.app);
        fold_store(&mut base, &mut report, &mut notice_shown, &opened);
        store = Some(opened);

        record(StartupStep::MergeManifest);
        let scan = super::env::scan_environment(&base.app, &manifest, &env);
        fold_environment(&mut base, &scan.values);
        if !scan.unmatched.is_empty() {
            // Not a `tracing::warn!`: no subscriber exists yet at step 3, so
            // it would go nowhere. The matching doctor finding is the
            // `telemetry.env` check's own job — it derives it from
            // `report.unmatched_env`, so pushing one here would double-report.
            eprintln!(
                "telemetry: ignoring {} unrecognised environment variable(s): {}",
                scan.unmatched.len(),
                scan.unmatched.join(", ")
            );
        }
        report.unmatched_env = scan.unmatched;
    }

    // 4. One pure resolution, once per process.
    record(StartupStep::Resolve);
    let mut policy = super::policy::resolve_policy(base);

    // The Install id is minted *after* the decision, by filling in the field
    // the resolution left empty — not by resolving twice. Minting is a write
    // to the person's disk, so it happens only once something is actually
    // being sent: someone who never turns telemetry on never acquires a
    // persistent identifier. (`resolve_policy` already clears the id under
    // anonymous attribution, which covers the unavailable-store case.)
    if policy.level > TelemetryLevel::Off
        && policy.attribution != Attribution::Anonymous
        && policy.install_id.is_none()
    {
        if let Some(store) = store.as_ref() {
            policy.install_id = store.ensure_install_id();
        }
    }

    // 5. Freeze. Everything below holds this `Arc`; nothing can mutate it.
    record(StartupStep::FreezePolicy);
    let policy = Arc::new(policy);

    // 6. Providers and the export boundary.
    let mut handle = None;
    let mut guard = None;
    if policy.exports() {
        record(StartupStep::BuildProviders);
        if let Some((live, built)) = super::init::init_from_policy(policy.clone(), service) {
            handle = Some(live);
            guard = Some(built.with_flush_budget(flush_budget_for(&policy)));
        }
    }

    // 7. The subscriber, after the providers because the OTel layer needs a
    //    live tracer to wrap.
    if let Some(guard) = guard.as_ref() {
        record(StartupStep::InstallSubscriber);
        report.subscriber = install_subscriber(guard, &mut report.findings);
    }

    // 8. The notice, before dispatch so a person reads it above the command's
    //    own output rather than under it.
    record(StartupStep::ShowNotice);
    let notice = match store.as_ref() {
        Some(store) => show_notice(&policy, notice_shown, surface, stderr_is_tty, store),
        // No store means a kill switch fired at step 1, and
        // `notice_decision` skips on a kill switch anyway: someone who set
        // `DO_NOT_TRACK` has already answered the question the notice asks.
        // Deciding that here, where both arms are reachable, keeps
        // `show_notice` total -- passing an `Option` inwards would leave it
        // with a no-store arm that no test could honestly reach.
        None => None,
    };

    // 9. The panic hook, last of the setup: a panic *during* startup should be
    //    reported by whatever hook was already installed, not by a half-built
    //    one that names a provider it never finished building.
    let mut panic_hook = false;
    if let Some(handle) = handle.as_ref() {
        record(StartupStep::InstallPanicHook);
        install_telemetry_panic_hook(handle.clone(), policy.clone());
        panic_hook = true;
    }

    // 10. The command itself, which is the caller's.
    record(StartupStep::Dispatch);

    StartupResult {
        policy,
        report: Arc::new(report),
        handle,
        guard,
        notice,
        panic_hook,
    }
}

/// The kill-switch probe, reading the snapshot rather than `std::env`.
fn detect_kill_switch_in(app: &str, env: &[(String, String)]) -> Option<KillSwitch> {
    super::policy::detect_kill_switch(app, &|key| {
        env.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    })
}

/// Fold the stored settings into the inputs, and the store's own health into
/// the report. A store that cannot be opened is a value, never an abort:
/// `store_available: false` is what degrades attribution to anonymous and
/// what the `telemetry.store` doctor check reports.
fn fold_store(
    base: &mut TelemetryInputs,
    report: &mut StartupReport,
    notice_shown: &mut Option<TelemetryLevel>,
    store: &TelemetryStore,
) {
    let state = store.state().clone();
    base.store_available = state.is_ready();
    base.store_error = state.reason().map(str::to_string);
    report.store = state;

    let settings = store.settings();
    if let Some(level) = settings.level {
        base.level.config_file = Some(level);
    }
    if let Some(attribution) = settings.attribution {
        base.attribution = attribution;
    }
    if let Some(endpoint) = settings.endpoint.filter(|e| !e.trim().is_empty()) {
        base.endpoint = Some(endpoint);
        base.endpoint_source = Some(Layer::ConfigFile);
    }
    if let Some(install_id) = settings.install_id {
        base.install_id = Some(install_id);
    }
    for (probe, enabled) in settings.probes {
        if enabled {
            base.disabled_probes.remove(&probe);
        } else {
            base.disabled_probes.insert(probe);
        }
    }
    *notice_shown = settings.notice_shown;
}

/// Fold the environment layer, which `scan_environment` has already typed and
/// keyed by dotted manifest path.
fn fold_environment(
    base: &mut TelemetryInputs,
    values: &serde_json::Map<String, serde_json::Value>,
) {
    // `scan_environment` only ever inserts leaves whose dotted path is
    // `telemetry` or `telemetry.<something>`, and `with_telemetry_section`
    // refuses an application manifest that already owns a bare `telemetry`
    // key -- so the first form cannot reach a real app. Stating that as a
    // filter rather than an escape inside the loop keeps the invariant where
    // a reader looks for it, and leaves no in-loop branch that no test could
    // honestly reach.
    let telemetry_keys = values
        .iter()
        .filter_map(|(path, value)| Some((path.strip_prefix("telemetry.")?, value)));
    for (key, value) in telemetry_keys {
        match key {
            "level" => {
                if let Some(level) = value
                    .as_str()
                    .and_then(|s| s.parse::<TelemetryLevel>().ok())
                {
                    base.level.environment = Some(level);
                }
            }
            "attribution" => {
                if let Some(attribution) =
                    value.as_str().and_then(|s| s.parse::<Attribution>().ok())
                {
                    base.attribution = attribution;
                }
            }
            "endpoint" => {
                if let Some(endpoint) = value.as_str().filter(|e| !e.trim().is_empty()) {
                    base.endpoint = Some(endpoint.to_string());
                    base.endpoint_source = Some(Layer::Environment);
                }
            }
            // `install_id` and `notice_shown` are `local_only` and
            // `protected`: the store owns them, and an environment variable
            // that could rewrite an Install's identity would defeat both
            // flags. They are leaves of the published manifest, so
            // `scan_environment` matches them; they are ignored here rather
            // than left to fall through into the probe-switch arm.
            "install_id" | "notice_shown" => {}
            _ => {
                if let (Some(probe), Some(enabled)) =
                    (key.strip_suffix(".enabled"), value.as_bool())
                {
                    if enabled {
                        base.disabled_probes.remove(probe);
                    } else {
                        base.disabled_probes.insert(probe.to_string());
                    }
                }
            }
        }
    }
}

/// An end-user Install gets a bounded flush so a slow or dead collector can
/// never hold a person's terminal; a Service flushes fully, because a dropped
/// trace there is a gap in someone's incident timeline.
fn flush_budget_for(policy: &TelemetryPolicy) -> Option<std::time::Duration> {
    match policy.deployment {
        Deployment::EndUser { .. } => Some(super::guard::END_USER_FLUSH_BUDGET),
        Deployment::Service => None,
    }
}

/// Install the composed subscriber, and fall back to the write-once slot that
/// [`crate::init_default_logging`] left behind when the process already owns
/// a subscriber of its own.
fn install_subscriber(
    guard: &TelemetryGuard,
    findings: &mut Vec<DoctorFinding>,
) -> SubscriberOutcome {
    let outcome = super::subscriber::install_telemetry_subscriber(guard);
    if outcome != SubscriberOutcome::ForeignSubscriber {
        return outcome;
    }
    // An app that called `init_default_logging()` is not a foreign
    // subscriber — it is *this* framework's, holding a slot for exactly this
    // layer. Fill it and the traces flow after all.
    let layer = super::init::otel_layer::<tracing_subscriber::Registry>(guard);
    if super::subscriber::attach_otel_layer_globally(Box::new(layer)) {
        return SubscriberOutcome::Installed;
    }
    super::subscriber::warn_once_foreign_subscriber(&|line| eprintln!("{line}"));
    findings.push(super::subscriber::foreign_subscriber_finding());
    SubscriberOutcome::ForeignSubscriber
}

/// Decide the notice, and record what was announced so the next run is quiet.
fn show_notice(
    policy: &TelemetryPolicy,
    notice_shown: Option<TelemetryLevel>,
    surface: Surface,
    stderr_is_tty: bool,
    store: &TelemetryStore,
) -> Option<String> {
    match super::notice::notice_decision(policy, notice_shown, surface, stderr_is_tty) {
        super::notice::NoticeDecision::Show {
            text,
            announced_level,
        } => {
            // Deliberately swallowed. A store that cannot be written is
            // already reported by the `telemetry.store` doctor check, and
            // per the spec an unwritable store means the notice prints
            // every run — which is the honest outcome, not a failure to
            // hand back to a person running an unrelated command.
            let _ = store.mutate(|s| s.notice_shown = Some(announced_level));
            Some(text)
        }
        super::notice::NoticeDecision::Skip(_) => None,
    }
}

/// Report panics through the probe catalog.
///
/// Deliberately not `Telemetry::event`: that collapses the whole attribute
/// slice into one `Debug`-formatted field, which would carry the panic
/// message past the key-level redactor that is supposed to hold it back to
/// `debug`. Two explicit call sites, with the message only on the second one,
/// keep the boundary able to see what it is deciding about.
///
/// `cli.probe` is not decoration: `redact_span` drops any event whose
/// `probe_of` names nothing, so an event without it never leaves the process.
fn install_telemetry_panic_hook(
    handle: Arc<dyn Telemetry + Send + Sync>,
    policy: Arc<TelemetryPolicy>,
) {
    super::panic::install_panic_hook(move |record| {
        if !policy.effective("cli.panic") {
            return;
        }
        // No attributes: every label in `METRIC_LABEL_ALLOWLIST` describes a
        // command, a surface or an outcome, and none of them describes a
        // crash. A location would be unbounded cardinality besides.
        handle.counter(super::probes::metrics::PANICS).add(1, &[]);
        let location = record.location.unwrap_or_default();
        match record
            .message
            .filter(|_| policy.effective("cli.panic.message"))
        {
            Some(message) => tracing::error!(
                cli.probe = "cli.panic",
                panic.location = %location,
                panic.message = %message,
                "the application panicked"
            ),
            None => tracing::error!(
                cli.probe = "cli.panic",
                panic.location = %location,
                "the application panicked"
            ),
        }
    });
}
