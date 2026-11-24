// "Strong" noise suppression, in the browser's audio thread: RNNoise (crates/denoise, built to
// natter_denoise.wasm) cleans the microphone in 10 ms frames at 48 kHz. glue.js (`denoised`) puts this
// between the mic and the call, and hands over the module's bytes; it has no imports.
const N = 480;

class Denoise extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.x = new WebAssembly.Instance(new WebAssembly.Module(options.processorOptions.wasm), {}).exports;
    this.st = this.x.dn_new();
    this.views();
    this.fill = 0;
    // Output queue, primed with one frame of silence so it never runs dry: 10 ms of delay.
    this.q = new Float32Array(N * 4);
    this.r = 0;
    this.w = N;
  }

  // (Again whenever the module's memory grows: that detaches the old views.)
  views() {
    this.mem = this.x.memory.buffer;
    this.inp = new Float32Array(this.mem, this.x.dn_input(this.st), N);
    this.out = new Float32Array(this.mem, this.x.dn_output(this.st), N);
  }

  process(inputs, outputs) {
    const i = inputs[0] && inputs[0][0];
    const o = outputs[0][0];
    if (!i) {
      o.fill(0);
      return true;
    }
    const L = this.q.length;
    for (let k = 0; k < i.length; k++) {
      this.inp[this.fill++] = i[k] * 32768;
      if (this.fill === N) {
        this.x.dn_process(this.st);
        if (this.x.memory.buffer !== this.mem) this.views();
        for (let j = 0; j < N; j++) {
          this.q[this.w] = this.out[j] / 32768;
          this.w = (this.w + 1) % L;
        }
        this.fill = 0;
      }
    }
    for (let k = 0; k < o.length; k++) {
      o[k] = this.q[this.r];
      this.r = (this.r + 1) % L;
    }
    return true;
  }
}

registerProcessor("natter-denoise", Denoise);
