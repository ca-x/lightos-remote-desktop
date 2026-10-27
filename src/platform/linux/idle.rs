//! Connection-scoped idle inhibition for desktops owned by this service. No
//! user settings are changed, and physical sessions never enter this path.
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
pub(super) struct Inhibitor {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Inhibitor {
    pub(super) fn start(address: String) -> Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = std::thread::Builder::new()
            .name("owned-desktop-idle".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(run(address, stop))
                })();
                if let Err(error) = result {
                    tracing::warn!(%error,"owned desktop idle inhibition stopped");
                }
            })?;
        Ok(Self {
            stopped,
            worker: Some(worker),
        })
    }
}
impl Drop for Inhibitor {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
async fn run(address: String, stopped: Arc<AtomicBool>) -> Result<()> {
    let bus = tokio::time::timeout(
        Duration::from_secs(2),
        zbus::connection::Builder::address(address.as_str())?.build(),
    )
    .await??;
    let dbus = zbus::fdo::DBusProxy::new(&bus).await?;
    let mut inhibited = HashMap::new();
    while !stopped.load(Ordering::Acquire) {
        for (name, path, interface) in [
            (
                "org.gnome.SessionManager",
                "/org/gnome/SessionManager",
                "org.gnome.SessionManager",
            ),
            (
                "org.freedesktop.ScreenSaver",
                "/ScreenSaver",
                "org.freedesktop.ScreenSaver",
            ),
            (
                "org.xfce.ScreenSaver",
                "/org/xfce/ScreenSaver",
                "org.xfce.ScreenSaver",
            ),
            (
                "org.freedesktop.PowerManagement",
                "/org/freedesktop/PowerManagement/Inhibit",
                "org.freedesktop.PowerManagement.Inhibit",
            ),
        ] {
            if stopped.load(Ordering::Acquire) {
                break;
            }
            let operation = async {
                let Ok(owner) = dbus.get_name_owner(name.try_into()?).await else {
                    inhibited.remove(name);
                    return Ok(());
                };
                if inhibited.get(name) == Some(&owner.to_string()) {
                    return Ok(());
                }
                let api = zbus::Proxy::new(&bus, owner.as_str(), path, interface).await?;
                let _: u32 = if name == "org.gnome.SessionManager" {
                    api.call(
                        "Inhibit",
                        &(
                            "LightOS Remote Desktop",
                            0_u32,
                            "Service-owned virtual desktop",
                            8_u32,
                        ),
                    )
                    .await?
                } else {
                    api.call(
                        "Inhibit",
                        &("LightOS Remote Desktop", "Service-owned virtual desktop"),
                    )
                    .await?
                };
                inhibited.insert(name, owner.to_string());
                tracing::info!(service = name, "owned desktop idle inhibition active");
                Ok::<_, anyhow::Error>(())
            };
            if let Err(error) = tokio::time::timeout(Duration::from_secs(2), operation)
                .await
                .context("idle inhibition timed out")
                .and_then(|r| r)
            {
                tracing::debug!(%error,service=name,"desktop does not provide idle inhibition");
            }
        }
        for _ in 0..20 {
            if stopped.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    // D-Bus removes these inhibitors when this connection closes, including
    // abnormal termination. No stale cookie or permanent desktop setting.
    Ok(())
}
