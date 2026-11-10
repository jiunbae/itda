// Itda: the remote machine's sound (QEMU Audio: S16LE, 48 kHz, stereo) through Web Audio.
// Scheduled AudioBufferSourceNodes rather than an AudioWorklet, which needs a secure context
// (the web viewer is plain HTTP on a LAN or tailnet). Each chunk starts where the last one
// ends; a short lead absorbs network jitter, and a chunk that would start more than MAX_AHEAD
// from now is dropped, so the sound never drifts behind the picture.
export const RATE = 48000, LEAD = 0.08, MAX_AHEAD = 0.3;

// When to start a chunk, given where the last one ends: null drops it.
export function schedule(next, now) {
    if (next < now + 0.005) return now + LEAD; // the first chunk, or ran dry: buffer again
    if (next - now > MAX_AHEAD) return null; // a backlog (a stalled link, a hidden tab)
    return next;
}

// S16LE interleaved stereo into two float channels (`bytes` may start at any offset).
export function decode(bytes, left, right) {
    const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    for (let i = 0, o = 0; o + 4 <= bytes.byteLength; i++, o += 4) {
        left[i] = v.getInt16(o, true) / 32768;
        right[i] = v.getInt16(o + 2, true) / 32768;
    }
}

export class Player {
    constructor(Ctx = globalThis.AudioContext || globalThis.webkitAudioContext) {
        this.ctx = new Ctx({ latencyHint: 'interactive' });
        this.next = 0;
    }
    // Browsers start sound only after a click or key: false while it waits for one.
    get running() { return this.ctx.state === 'running'; }
    resume() { if (this.ctx.state === 'suspended') this.ctx.resume().catch(() => {}); }
    begin() { this.next = 0; }
    end() {}
    push(bytes) {
        const frames = bytes.byteLength >> 2;
        if (!frames || !this.running) return;
        const at = schedule(this.next, this.ctx.currentTime);
        if (at === null) return;
        const buf = this.ctx.createBuffer(2, frames, RATE); // (resampled to the device's rate)
        decode(bytes, buf.getChannelData(0), buf.getChannelData(1));
        const src = this.ctx.createBufferSource();
        src.buffer = buf;
        src.connect(this.ctx.destination);
        src.start(at);
        this.next = at + frames / RATE;
    }
    close() { this.ctx.close().catch(() => {}); }
}
