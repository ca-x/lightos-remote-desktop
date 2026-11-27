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
    /// Independent of the physical screen/input grant. Only opt-in hosts may
    /// keep the selection service running using this capability.
    pub virtual_available: bool,
    pub request_mode: &'static str,
    pub reason_code: &'static str,
    pub settings: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<crate::backend::BackendKind>,
    #[cfg(target_os = "windows")]
    pub lock_screen_authorized: bool,
    #[cfg(target_os = "windows")]
    pub lock_screen_setup_required: bool,
    #[cfg(target_os = "windows")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_screen_request_result: Option<&'static str>,
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
            virtual_available: false,
            request_mode: mode,
            reason_code: code,
            settings,
            backend: None,
            #[cfg(target_os = "windows")]
            lock_screen_authorized: false,
            #[cfg(target_os = "windows")]
            lock_screen_setup_required: true,
            #[cfg(target_os = "windows")]
            lock_screen_request_result: None,
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
    let report = physical_report(config, request)?;
    #[cfg(target_os = "linux")]
    {
        let mut report = report;
        if config.on_demand_desktops() {
            report.virtual_available = super::current().supports_virtual(config);
            report.can_start |= report.virtual_available;
        }
        Ok(report)
    }
    #[cfg(not(target_os = "linux"))]
    Ok(report)
}

fn physical_report(config: &Config, request: bool) -> Result<PermissionReport> {
    #[cfg(target_os = "macos")]
    {
        let _ = config;
        Ok(match super::macos::permissions(request) {
            Ok(permissions) => {
                let mut report = PermissionReport::new(
                    permissions,
                    "system",
                    "permission_required",
                    vec![
                        "screen",
                        "input",
                        super::macos::input_permission().setting(),
                    ],
                );
                if super::macos::session_locked() {
                    report.reason_code = if report.can_start {
                        "lock_screen_ready"
                    } else {
                        "session_locked"
                    };
                }
                report
            }
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
        let selected = match selection.require_available() {
            Ok(selected) => selected,
            Err(error) => {
                let code = selection
                    .candidates
                    .iter()
                    .rev()
                    .find_map(|candidate| candidate.reason_code)
                    .unwrap_or("capture_unavailable");
                return Ok(PermissionReport::unavailable(error.to_string(), code));
            }
        };
        let locked = match crate::backend::guarded::require_attachable(&target, selected) {
            Ok(locked) => locked,
            Err(error) => {
                let code = error
                    .downcast_ref::<crate::backend::CapabilityError>()
                    .map_or("session_unavailable", |error| error.code);
                return Ok(PermissionReport::unavailable(format!("{error:#}"), code));
            }
        };
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
                let mut result = physical_report(config, false)?;
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
            if locked { "lock_screen_ready" } else { "ready" },
            vec![],
        );
        report.backend = selection.selected;
        Ok(report)
    }
}
