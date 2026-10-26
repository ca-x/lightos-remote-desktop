use super::*;
use libpulse_binding::{
    context::{Context, FlagSet as ContextFlags, State as ContextState},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format, Spec},
    stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream},
};
fn step(mainloop: &mut Mainloop) -> Result<()> {
    match mainloop.iterate(false) {
        IterateResult::Success(_) => Ok(()),
        _ => bail!("Pulse mainloop stopped"),
    }
}

pub(super) fn capture(
    server: Option<&str>,
    source: &str,
    stop: &AtomicBool,
    packets: &PacketQueue,
    mut packetizer: Packetizer,
) -> Result<()> {
    let mut mainloop = Mainloop::new().context("create Pulse mainloop")?;
    let mut context =
        Context::new(&mainloop, "LightOS remote audio").context("create Pulse context")?;
    context
        .connect(server, ContextFlags::NOAUTOSPAWN, None)
        .context("connect Pulse server")?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            step(&mut mainloop)?;
            match context.get_state() {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => bail!("Pulse context failed"),
                _ => {}
            }
            ensure!(Instant::now() < deadline, "Pulse context timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        let spec = Spec {
            format: Format::F32le,
            channels: 2,
            rate: 48_000,
        };
        let mut stream = Stream::new(&mut context, "Desktop monitor", &spec, None)
            .context("create Pulse stream")?;
        let result = (|| {
            let attr = BufferAttr {
                maxlength: (PACKET_BYTES * 10) as u32,
                tlength: u32::MAX,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: PACKET_BYTES as u32,
            };
            stream
                .connect_record(Some(source), Some(&attr), StreamFlags::ADJUST_LATENCY)
                .context("record Pulse monitor")?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stop.load(Ordering::Acquire) {
                step(&mut mainloop)?;
                ensure!(
                    !matches!(
                        context.get_state(),
                        ContextState::Failed | ContextState::Terminated
                    ),
                    "Pulse context lost"
                );
                match stream.get_state() {
                    StreamState::Ready => {
                        for _ in 0..32 {
                            match stream.peek().context("peek Pulse audio")? {
                                PeekResult::Empty => break,
                                PeekResult::Data(data) => {
                                    ensure!(data.len() <= 1024 * 1024, "Pulse fragment too large");
                                    packetizer.push(data, packets)?;
                                }
                                PeekResult::Hole(length) => {
                                    ensure!(length <= 1024 * 1024, "Pulse hole too large");
                                    packetizer.silence(length, packets)?;
                                }
                            }
                            stream.discard().context("discard Pulse fragment")?;
                        }
                    }
                    StreamState::Failed | StreamState::Terminated => bail!("Pulse stream failed"),
                    _ => ensure!(Instant::now() < deadline, "Pulse stream timed out"),
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })();
        let _ = stream.disconnect();
        result
    })();
    context.disconnect();
    result
}
