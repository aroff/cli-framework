use crate::app::context::AppContext;
use crate::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding};
use crate::doctor::DoctorError;
use std::sync::Arc;
use tokio::task::JoinSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorReport {
    pub findings: Vec<DoctorFinding>,
    pub ok: usize,
    pub warnings: usize,
    pub errors: usize,
    pub skipped: usize,
}

impl DoctorReport {
    pub fn from_findings(findings: Vec<DoctorFinding>) -> Self {
        let mut ok = 0;
        let mut warnings = 0;
        let mut errors = 0;
        let mut skipped = 0;
        for f in &findings {
            match f.severity {
                CheckSeverity::Ok => ok += 1,
                CheckSeverity::Warning => warnings += 1,
                CheckSeverity::Error => errors += 1,
                CheckSeverity::Skipped => skipped += 1,
            }
        }
        DoctorReport {
            findings,
            ok,
            warnings,
            errors,
            skipped,
        }
    }
}

pub struct DoctorRunner {
    checks: Vec<Arc<dyn DoctorCheck>>,
}

impl DoctorRunner {
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    pub fn from_checks(checks: Vec<Arc<dyn DoctorCheck>>) -> Self {
        Self { checks }
    }

    pub fn register(&mut self, check: Arc<dyn DoctorCheck>) -> Result<(), DoctorError> {
        let id = check.id();
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || id.is_empty()
        {
            return Err(DoctorError::DuplicateCheckId(format!(
                "invalid id '{}': must be non-empty kebab-case",
                id
            )));
        }
        if self.checks.iter().any(|c| c.id() == id) {
            return Err(DoctorError::DuplicateCheckId(id.to_string()));
        }
        self.checks.push(check);
        Ok(())
    }

    pub async fn run_all(&self, ctx: &dyn AppContext) -> DoctorReport {
        let n = self.checks.len();
        if n == 0 {
            return DoctorReport::from_findings(vec![]);
        }

        let mut join_set: JoinSet<(usize, DoctorFinding)> = JoinSet::new();
        for (idx, check) in self.checks.iter().enumerate() {
            let check = Arc::clone(check);
            let future = check.run(ctx);
            join_set.spawn(async move { (idx, future.await) });
        }

        let mut slots: Vec<Option<DoctorFinding>> = vec![None; n];
        let mut panic_messages: Vec<String> = Vec::new();

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((idx, finding)) => {
                    slots[idx] = Some(finding);
                }
                Err(e) => {
                    let msg = extract_panic_message(e);
                    panic_messages.push(msg);
                }
            }
        }

        let mut panic_iter = panic_messages.into_iter();
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                let panic_str = panic_iter.next().unwrap_or_else(|| "panicked".to_string());
                let check = &self.checks[i];
                *slot = Some(DoctorFinding {
                    check_id: check.id().to_string(),
                    title: check.title().to_string(),
                    severity: CheckSeverity::Error,
                    message: format!("DR002: check '{}' panicked: {}", check.id(), panic_str),
                    detail: None,
                    remediation: None,
                });
            }
        }

        let findings: Vec<DoctorFinding> = slots
            .into_iter()
            .map(|s| s.expect("all slots filled after JoinSet completion"))
            .collect();

        DoctorReport::from_findings(findings)
    }

    pub async fn run_filtered(&self, ctx: &dyn AppContext, ids: &[&str]) -> DoctorReport {
        let n = ids.len();
        let mut slots: Vec<Option<DoctorFinding>> = vec![None; n];
        let mut join_set: JoinSet<(usize, DoctorFinding)> = JoinSet::new();

        for (i, id) in ids.iter().enumerate() {
            match self.checks.iter().find(|c| c.id() == *id) {
                Some(check) => {
                    let check = Arc::clone(check);
                    let future = check.run(ctx);
                    join_set.spawn(async move { (i, future.await) });
                }
                None => {
                    slots[i] = Some(DoctorFinding {
                        check_id: id.to_string(),
                        title: format!("Unknown check '{}'", id),
                        severity: CheckSeverity::Error,
                        message: format!("DR003: unknown check id '{}'", id),
                        detail: None,
                        remediation: Some(
                            "Run 'doctor' (without --check) to see all available checks."
                                .to_string(),
                        ),
                    });
                }
            }
        }

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((i, finding)) => {
                    slots[i] = Some(finding);
                }
                Err(e) => {
                    let panic_str = extract_panic_message(e);
                    for (i, slot) in slots.iter_mut().enumerate() {
                        if slot.is_none() {
                            if let Some(check) = self.checks.iter().find(|c| c.id() == ids[i]) {
                                *slot = Some(DoctorFinding {
                                    check_id: check.id().to_string(),
                                    title: check.title().to_string(),
                                    severity: CheckSeverity::Error,
                                    message: format!(
                                        "DR002: check '{}' panicked: {}",
                                        check.id(),
                                        panic_str
                                    ),
                                    detail: None,
                                    remediation: None,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }

        let findings: Vec<DoctorFinding> = slots.into_iter().flatten().collect();
        DoctorReport::from_findings(findings)
    }
}

impl Default for DoctorRunner {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_panic_message(e: tokio::task::JoinError) -> String {
    if e.is_panic() {
        let p = e.into_panic();
        if let Some(s) = p.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = p.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        }
    } else {
        "task was cancelled".to_string()
    }
}
