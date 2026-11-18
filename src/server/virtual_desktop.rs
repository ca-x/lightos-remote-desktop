//! Retain a user-selected desktop across browser reconnects, not across host shutdown.
use super::*;

#[derive(Clone)]
pub(super) struct RetainedDesktop {
    backend: Arc<dyn DesktopBackend>,
    displays: Arc<dyn DisplayController>,
    session: Arc<Mutex<Box<dyn SessionLifecycle>>>,
    audio: Arc<dyn AudioCaptureFactory>,
    output: Option<String>,
    outputs: Vec<DisplayOutput>,
}

impl RetainedDesktop {
    fn new(host: HostSession) -> Self {
        Self {
            backend: host.backend,
            displays: host.displays,
            session: Arc::new(Mutex::new(host.lifecycle)),
            audio: host.audio,
            output: host.output,
            outputs: host.outputs,
        }
    }

    pub(super) fn apply(self, state: &mut AppState) {
        state.audio = Arc::new(crate::audio::SessionAudio::new(
            self.audio,
            self.backend.clone(),
        ));
        state.backend = self.backend;
        state.displays = self.displays;
        state.session = self.session;
        state.output = self.output;
        state.outputs = self.outputs;
        state.desktop = DesktopMode::Virtual;
        state.transient_backend = false;
    }

    fn healthy(&self) -> bool {
        !self.backend.is_stopped()
            && self
                .session
                .lock()
                .is_ok_and(|mut session| session.healthy())
    }
}

#[derive(Default)]
pub(super) struct VirtualDesktop {
    retained: Mutex<Option<RetainedDesktop>>,
    startup: Mutex<Option<Arc<AtomicBool>>>,
    stopped: AtomicBool,
}

impl VirtualDesktop {
    pub(super) fn retain(&self, state: &AppState) {
        *self
            .retained
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(RetainedDesktop {
            backend: state.backend.clone(),
            displays: state.displays.clone(),
            session: state.session.clone(),
            audio: state.audio.clone(),
            output: state.output.clone(),
            outputs: state.outputs.clone(),
        });
    }

    pub(super) fn acquire(
        &self,
        host: &dyn HostPlatform,
        config: &Config,
        cancelled: Arc<AtomicBool>,
    ) -> anyhow::Result<RetainedDesktop> {
        use anyhow::ensure;
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure!(
            !cancelled.load(Ordering::Acquire) && !self.stopped.load(Ordering::Acquire),
            "virtual desktop startup cancelled or service stopping"
        );
        if let Some(context) = retained.as_ref() {
            ensure!(
                context.healthy(),
                "虚拟桌面已停止，请关闭并重新开启远程桌面；不会静默创建空白桌面"
            );
            return Ok(context.clone());
        }
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "desktop service is stopping"
        );
        *self
            .startup
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(cancelled.clone());
        // Shutdown cancels startup before waiting for the retained-session lock.
        // Recheck after publishing the flag to close the admission race.
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "desktop service is stopping"
        );
        let context = RetainedDesktop::new(host.open_virtual(config, cancelled.clone())?);
        if cancelled.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
            context.backend.stop();
            anyhow::bail!("virtual desktop startup cancelled");
        }
        *retained = Some(context.clone());
        Ok(context)
    }

    pub(super) fn discard_cancelled_startup(&self, cancelled: &Arc<AtomicBool>) {
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut startup = self
            .startup
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if startup
            .as_ref()
            .is_some_and(|flag| Arc::ptr_eq(flag, cancelled))
        {
            if let Some(context) = retained.take() {
                context.backend.stop();
            }
            startup.take();
        }
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(cancelled) = self
            .startup
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(context) = self
            .retained
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            context.backend.stop();
        }
    }
}

impl Drop for VirtualDesktop {
    fn drop(&mut self) {
        if let Some(context) = self
            .retained
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            context.backend.stop();
        }
    }
}
