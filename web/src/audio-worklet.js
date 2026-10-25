class RemoteAudioProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.capacity = 5760;
    this.left = new Float32Array(this.capacity);
    this.right = new Float32Array(this.capacity);
    this.readIndex = 0;
    this.writeIndex = 0;
    this.available = 0;
    this.phase = 0;
    this.playing = false;
    this.dropped = 0;
    this.underruns = 0;
    this.reportFrames = 0;
    this.port.onmessage = ({ data }) => {
      if (data?.type === "reset") {
        this.readIndex = this.writeIndex = this.available = this.phase = 0;
        this.playing = false;
        return;
      }
      if (!data?.port) return;
      data.port.onmessage = ({ data: packet }) => {
        this.receive(packet);
        data.port.postMessage({ type: "consumed" });
      };
      data.port.start();
    };
  }

  receive(packet) {
    const { left, right } = packet ?? {};
    if (!(left instanceof Float32Array) || !(right instanceof Float32Array)
      || left.length !== right.length || !left.length || left.length > this.capacity) return;
    if (this.available + left.length > this.capacity) {
      const discard = Math.min(this.available, this.available + left.length - 1920);
      this.readIndex = (this.readIndex + discard) % this.capacity;
      this.available -= discard;
      this.dropped += discard;
      this.phase = 0;
    }
    for (let index = 0; index < left.length; index += 1) {
      this.left[this.writeIndex] = Number.isFinite(left[index]) ? Math.max(-1, Math.min(1, left[index])) : 0;
      this.right[this.writeIndex] = Number.isFinite(right[index]) ? Math.max(-1, Math.min(1, right[index])) : 0;
      this.writeIndex = (this.writeIndex + 1) % this.capacity;
    }
    this.available += left.length;
  }

  process(inputs, outputs) {
    const output = outputs[0];
    if (!output?.length) return true;
    const frames = output[0].length;
    if (!this.playing && this.available >= 1920) this.playing = true;
    for (let index = 0; index < frames; index += 1) {
      if (!this.playing || this.available < 1) {
        if (this.playing) this.underruns += 1;
        this.playing = false;
        this.phase = 0;
        for (const channel of output) channel[index] = 0;
        continue;
      }
      const next = this.available > 1 ? (this.readIndex + 1) % this.capacity : this.readIndex;
      output[0][index] = this.left[this.readIndex] * (1 - this.phase) + this.left[next] * this.phase;
      if (output[1]) output[1][index] = this.right[this.readIndex] * (1 - this.phase) + this.right[next] * this.phase;
      this.phase += 48000 / sampleRate;
      const consumed = Math.min(this.available, Math.floor(this.phase));
      this.phase -= consumed;
      this.readIndex = (this.readIndex + consumed) % this.capacity;
      this.available -= consumed;
    }
    this.reportFrames += frames;
    if (this.reportFrames >= sampleRate) {
      this.reportFrames = 0;
      this.port.postMessage({ bufferMs: this.available / 48, underruns: this.underruns, dropped: this.dropped });
    }
    return true;
  }
}

registerProcessor("remote-audio", RemoteAudioProcessor);
