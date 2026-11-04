//! Automatic permission diagnostics are read-only. An explicit request can
//! prepare and persist a physical Portal grant without opening a viewer page.
use super::{PermissionState, Permissions};
use crate::config::Config;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct PermissionReport {
    #[serde(flatten)]
    pub permissions: Permissions,
    pub can_start: bool,
    pub request_mode: &'static str,
    pub reason_code: &'static str,
    pub settings: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<crate::backend::BackendKind>,
}
impl PermissionReport {
    fn new(
        permissions: Permissions,
        mode: &'static str,
        code: &'static str,
        settings: Vec<&'static str>,
    ) -> Self {
        let can_start = matches!(permissions.screen, PermissionState::Granted)
            && matches!(permissions.input, PermissionState::Granted);
        Self {
            permissions,
            can_start,
            request_mode: mode,
            reason_code: code,
            settings,
            backend: None,
        }
    }
    pub(super) fn unavailable(reason: String, code: &'static str) -> Self {
        Self::new(
            Permissions {
                screen: PermissionState::Denied,
                input: PermissionState::Denied,
                reason: Some(reason),
            },
            "none",
            code,
            vec![],
        )
    }
    #[cfg(target_os = "windows")]
    pub(super) fn windows(
        permissions: Permissions,
        mode: &'static str,
        code: &'static str,
    ) -> Self {
        let settings = if code == "permission_required" || code == "system_policy_denied" {
            vec!["screen"]
        } else {
            vec![]
        };
        Self::new(permissions, mode, code, settings)
    }
}

pub fn report(config: &Config, request: bool) -> Result<PermissionReport> {
    #[cfg(target_os = "macos")]
    {
        let _ = config;
        Ok(match super::macos::permissions(request) {
            Ok(permissions) => PermissionReport::new(
                permissions,
                "system",
                "permission_required",
                vec!["screen", "input"],
            ),
            Err(error) => PermissionReport::unavailable(error.to_string(), "session_unavailable"),
        })
    }
    #[cfg(target_os = "windows")]
    {
        let _ = config;
        super::windows::permission_report(request)
    }
    #[cfg(target_os = "linux")]
    {
        let target = match crate::session::Session::physical_target(config) {
            Ok(target) => target,
            Err(error) => {
                return Ok(PermissionReport::unavailable(
                    error.to_string(),
                    "session_unavailable",
                ));
            }
        };
        let selection = match crate::backend::probe(&target, true, None) {
            Ok(selection) => selection,
            Err(error) => {
                return Ok(PermissionReport::unavailable(
                    error.to_string(),
                    "capture_unavailable",
                ));
            }
        };
        if let Err(error) = selection.require_available() {
            let code = selection
                .candidates
                .iter()
                .rev()
                .find_map(|candidate| candidate.reason_code)
                .unwrap_or("capture_unavailable");
            return Ok(PermissionReport::unavailable(error.to_string(), code));
        }
        if selection.requires_consent() {
            let approved = selection.candidates.iter().any(|candidate| {
                Some(candidate.backend) == selection.selected && candidate.authorized == Some(true)
            });
            if request && !approved {
                let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let (notice, _) = tokio::sync::oneshot::channel();
                let outcome = super::current().open_physical(config, None, cancelled, notice);
                let error = match outcome {
                    Ok(host) => {
                        // Consent is retained in private storage. No video or
                        // input is sent to a remote viewer during preparation.
                        host.backend.stop();
                        drop(host);
                        None
                    }
                    Err(error) => Some(format!("{error:#}")),
                };
                let mut result = report(config, false)?;
                if let Some(error) = error {
                    result.reason_code = "authorization_failed";
                    result.permissions.reason = Some(error);
                } else if !matches!(result.permissions.screen, PermissionState::Granted)
                    || !matches!(result.permissions.input, PermissionState::Granted)
                {
                    result.reason_code = "authorization_not_saved";
                    result.permissions.reason = Some("The system did not retain the approval; authorization is still required when connecting".into());
                }
                return Ok(result);
            }
            let state = if approved {
                PermissionState::Granted
            } else {
                PermissionState::NeedsUserAction
            };
            let mut report = PermissionReport::new(
                Permissions {
                    screen: state.clone(),
                    input: state,
                    reason: (!approved).then(|| {
                        "Approve screen and input access on the controlled computer".into()
                    }),
                },
                "system",
                if approved {
                    "ready"
                } else {
                    "connection_consent"
                },
                vec![],
            );
            report.can_start = true;
            report.backend = selection.selected;
            return Ok(report);
        }
        let mut report = PermissionReport::new(
            Permissions {
                screen: PermissionState::Granted,
                input: PermissionState::Granted,
                reason: None,
            },
            "none",
            "ready",
            vec![],
        );
        report.backend = selection.selected;
        Ok(report)
    }
}
