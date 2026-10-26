# Independent audio transport

The browser uses a second authenticated, same-origin WebSocket at `./audio`.
The desktop ready message advertises an `audio_session` identifier. This is a
connection-association identifier, not an authentication credential. Basic Auth
and the existing WebSocket Origin checks remain mandatory on both endpoints.

Only one audio connection may attach to the current controller session. Closing
the controller, switching desktops or stopping the server invalidates its audio
session and closes the associated stream. Closing audio must not close video or
release remote input. An older server without the capability retains the legacy
single-socket audio path.

`AudioCaptureFactory` supplies a native source: Linux private PulseAudio,
Windows WASAPI loopback, or macOS ScreenCaptureKit system audio. Native desktop
permissions and session checks apply to audio as well as video. Streams belong
to one privacy epoch; lock or permission transitions discard buffered packets.
Windows/macOS runtime acceptance remains experimental; see
[native host support](cross-platform-host.md).

Audio capture stays on its native worker. Linux Pulse initialization runs on the
blocking pool; the independent async sender does not wait for video encoding or
render acknowledgements. Its four-packet queue drops stale capture packets;
a send timeout terminates a slow audio connection instead of accumulating an
unbounded application queue.

## Codec negotiation and playback

The desktop ready message advertises `audio_codecs: ["opus", "pcm"]`.
Capable browsers connect with `./audio?session=<id>&codec=opus`. Opus encodes
48 kHz stereo into 20 ms packets at 128 kbit/s CBR, complexity 5, on the native
capture thread. libopus is source-built and statically linked through the locked
`opus` / `opusic-sys` dependencies. There is no runtime codec download or FFmpeg.
Each binary message has marker 4, an eight-byte little-endian sender-relative
microsecond timestamp, and a raw Opus packet. The timestamp is not a synchronized
capture clock or an input-to-display latency measurement. `audio_ready` declares
the selected codec, sample rate and channels. Missing codec queries retain PCM
(marker 2 followed by interleaved float32 little-endian samples).

A dedicated browser Worker negotiates WebCodecs Opus support, decodes Opus or
converts legacy PCM, and transfers planar samples directly to an AudioWorklet
over a MessageChannel. Network-to-Worker and Worker-to-worklet transfers each
have a six-packet credit limit; excess packets/samples are discarded instead of
creating an unbounded application backlog. The worklet uses a preallocated stereo
ring, starts with 40 ms of samples, caps its ring at 120 ms and drops old samples
on overflow. It emits silence and re-prebuffers after underruns, and resamples if
the output device does not accept the requested 48 kHz context. These limits are
per stage, not a claim of a 120 ms end-to-end bound; network, TCP and proxy buffers
are outside the application's control.

Browsers without Opus decoding use PCM through the same Worker/worklet. Without
Worker or AudioWorklet, the existing bounded main-thread PCM player remains.
AudioWorklet needs a secure context (HTTPS or localhost). Failed initialization
or decoding stops only audio; it does not reconnect video. Initialization has a
five-second deadline and controller-generation checks. Mute, hiding the page,
desktop switching and teardown reclaim the Worker, worklet port and AudioContext.
All JS modules are embedded, same-origin, authenticated and support URL prefixes.

Opus payload is about 128 kbit/s, versus PCM's 3.07 Mbit/s; binary application
headers make Opus about 131.6 kbit/s before WebSocket/TCP overhead. This does not
change video resolution, quality, pacing or ACK window limits. Independent
connections still contend on shared links and proxies. Measure actual WAN and
browser behavior instead of treating codec or thread separation as proof of a fix.

## Diagnostics

Hover the FPS/RTT readout for recent encode/preparation, socket-send and render-ACK
P95 values from the server, and browser decode/draw P95. Server ping responses
include `video_metrics`, with bounded last-120-sample counts and P50/P95 values.
ACK timing includes sending, network/proxy traversal, browser scheduling and
render acknowledgement; it is not pure decode time. Socket-send completion and
loopback TCP ACKs are not proof that the browser received or displayed a frame.
Browser decode/draw starts when decoding is submitted, not at remote capture.
With worklet audio enabled, the tooltip also shows codec, ring-buffer duration,
underrun events and discarded source samples. Diagnostics do not enter React's
frame hot path. No clock subtraction between different machines is used.
