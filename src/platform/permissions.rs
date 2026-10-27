//! Permission diagnostics do not capture pixels or inject input. Linux Portal
//! consent belongs to the real connection, not to a discarded startup probe.
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
        let _ = request;
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
            return Ok(PermissionReport::unavailable(
                error.to_string(),
                "capture_unavailable",
            ));
        }
        if selection.requires_consent() {
            let mut report=PermissionReport::new(Permissions {
                screen:PermissionState::NeedsUserAction,input:PermissionState::NeedsUserAction,
                reason:Some("The system requests screen and input approval when a remote connection starts".into()),
            },"on_connection","connection_consent",vec![]);
            report.can_start = true;
            return Ok(report);
        }
        Ok(PermissionReport::new(
            Permissions {
                screen: PermissionState::Granted,
                input: PermissionState::Granted,
                reason: None,
            },
            "none",
            "ready",
            vec![],
        ))
    }
}
