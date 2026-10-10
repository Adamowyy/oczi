// Dictation, island side: the microphone is captured here and nowhere else.

import { Bridge } from "./bridge";

export interface ListenOptions {
  /** Current loudness, 0..1, for the level bars. */
  onLevel?: (level: number) => void;
  /** How long a pause has to last to end the recording. */
  silenceMs?: number;
  /** Quiet before the first word is expected — the user needs a moment to start. */
  graceMs?: number;
  /** Nothing said at all within this long: end with no samples rather than waiting. */
  noSpeechMs?: number;
  /** Hard stop, so a device that never reports silence cannot hold the encoder forever. */
  maxMs?: number;
}

/** Raised when there is no usable microphone: a card says so instead of a console error. */
export class NoMic extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NoMic";
  }
}

/** Below this RMS the room counts as quiet. Speech in a normal room is 10-100× this. */
const SILENCE_RMS = 0.012;
/** How much louder than the room a tick has to be to count as a voice. The room is measured
 *  during the first `graceMs`: a fan or a mechanical keyboard can sit above SILENCE_RMS, and
 *  treating that as speech is what kept a silent recording alive for the whole cap. */
const SPEECH_RATIO = 3;
/** One poll of the analyser. 50 ms is finer than the pause lengths being measured. */
const TICK_MS = 50;
/** What Whisper was trained on; asking the device for it directly means no resampling. */
const RATE = 16_000;
/** Frames per audio callback: about 128 ms of audio at 16 kHz. */
const BUFFER = 2048;

type Handle = { promise: Promise<Float32Array>; stop: () => void };

/** Start listening. The recording ends on silence, on `stop()`, on saying nothing at all, on
 *  a device that stops delivering audio, or at `maxMs`. Every one of those hands the promise
 *  back — a handle that never settles is what leaves dictation dead for the rest of the
 *  session, and it is why each of those paths says in the log which one it was. */
export function listen(opts: ListenOptions = {}): Handle {
  const silenceMs = opts.silenceMs ?? 1200;
  const graceMs = opts.graceMs ?? 500;
  const noSpeechMs = opts.noSpeechMs ?? 6000;
  const maxMs = opts.maxMs ?? 15_000;
  /** No audio callbacks for this long: the device has stopped talking to us. */
  const stallMs = 2000;

  let stream: MediaStream | null = null;
  let audio: AudioContext | null = null;
  let source: MediaStreamAudioSourceNode | null = null;
  let processor: ScriptProcessorNode | null = null;
  let sink: GainNode | null = null;
  let analyser: AnalyserNode | null = null;
  /** The analyser hands its samples to a plain `ArrayBuffer` view — typed explicitly,
   *  because a bare `Float32Array` also covers the shared-memory variant, which the
   *  analyser refuses. */
  let probe: Float32Array<ArrayBuffer> | null = null;
  let resolveOnce: ((samples: Float32Array) => void) | null = null;
  let timer = 0;
  let done = false;
  let elapsed = 0;
  let quietFor = 0;
  let spoke = false;
  /** The room's loudness, taken before anyone speaks, and the run of loud ticks. */
  let floor = 0;
  let loud = 0;
  let lastBlockAt = 0;
  const blocks: Float32Array[] = [];
  let frames = 0;

  /** Let go of the device. Called exactly once, from `end`. */
  const release = () => {
    window.clearInterval(timer);
    if (processor) processor.onaudioprocess = null;
    processor?.disconnect();
    source?.disconnect();
    analyser?.disconnect();
    sink?.disconnect();
    stream?.getTracks().forEach((t) => t.stop());
    void audio?.close();
    processor = null;
    source = null;
    analyser = null;
    sink = null;
    audio = null;
    stream = null;
  };

  /** The one way out: hand the promise its answer, with the audio collected so far or with
   *  nothing at all. `why` is what the log shows when a recording did not end the normal way.
   *
   *  `keep` is decided by whether a voice was heard at all, not by how the recording ended:
   *  a mis-click that is stopped by hand still holds several seconds of room, and running the
   *  engine over that costs three seconds and then answers with a marker or an invented
   *  sentence. Silence is not a question, so it never reaches the engine. */
  const end = (keep: boolean, why: string) => {
    if (done) return;
    done = true;
    const collected = frames;
    let samples = new Float32Array(0);
    if (keep && collected > 0) {
      samples = new Float32Array(collected);
      let at = 0;
      for (const block of blocks) {
        samples.set(block, at);
        at += block.length;
      }
    }
    blocks.length = 0;
    frames = 0;
    release();
    void Bridge.log(`mic  ${why} — ${(collected / RATE).toFixed(1)}s captured`);
    resolveOnce?.(samples);
  };

  const promise = (async () => {
    try {
      // Echo cancellation and noise suppression are on: the island plays sounds, and a
      // recording that starts right after one would otherwise carry it.
      stream = await navigator.mediaDevices.getUserMedia({
        audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
      });
    } catch (err) {
      throw new NoMic(String(err));
    }
    // Stopped while the device was opening.
    if (done) {
      stream.getTracks().forEach((t) => t.stop());
      stream = null;
      return new Float32Array(0);
    }

    audio = new AudioContext({ sampleRate: RATE });
    source = audio.createMediaStreamSource(stream);
    analyser = audio.createAnalyser();
    analyser.fftSize = 1024;
    probe = new Float32Array(analyser.fftSize);
    processor = audio.createScriptProcessor(BUFFER, 1, 1);
    processor.onaudioprocess = (e) => {
      if (done) return;
      lastBlockAt = performance.now();
      const data = e.inputBuffer.getChannelData(0);
      blocks.push(new Float32Array(data));
      frames += data.length;
    };
    // A processor only runs while it is connected to the destination, and a gain of zero
    // keeps the microphone out of the speakers while it does.
    sink = audio.createGain();
    sink.gain.value = 0;
    source.connect(analyser);
    source.connect(processor);
    processor.connect(sink);
    sink.connect(audio.destination);
    void Bridge.log(`mic  open rate=${audio.sampleRate}`);

    // Stopped between the device opening and the graph being wired up.
    if (done) return new Float32Array(0);
    lastBlockAt = performance.now();
    return await new Promise<Float32Array>((resolve) => {
      resolveOnce = resolve;
      timer = window.setInterval(() => {
        if (done || !analyser || !probe) return;
        // Nothing arrived from the audio thread: end rather than hold the device open for
        // ever. This is the guard that turns "frozen on listening" into a finished recording.
        if (performance.now() - lastBlockAt > stallMs) {
          end(true, "stalled");
          return;
        }
        analyser.getFloatTimeDomainData(probe);
        let sum = 0;
        for (let i = 0; i < probe.length; i++) sum += probe[i] * probe[i];
        const rms = Math.sqrt(sum / probe.length);
        opts.onLevel?.(Math.min(1, rms * 6));

        elapsed += TICK_MS;
        // The room first, measured before anyone can have spoken: this is the difference
        // between "a fan" and "a voice", and it is why a silent recording ends instead of
        // running to the cap while the microphone hears the machine.
        if (elapsed <= graceMs) {
          floor = Math.max(floor, rms);
          return;
        }
        const speechRms = Math.max(SILENCE_RMS * 1.5, floor * SPEECH_RATIO);
        // Two ticks in a row above it: one is a door, a click, or the keyboard.
        loud = rms > speechRms ? loud + 1 : 0;
        if (loud >= 2) spoke = true;
        quietFor = rms > SILENCE_RMS ? 0 : quietFor + TICK_MS;

        const longEnough = spoke && quietFor >= silenceMs;
        // Nothing said at all: end with nothing rather than sitting on the microphone. An
        // empty result is a mis-click, not a failure, and it must not reach the engine —
        // Whisper answers silence with a marker or an invented sentence, and either one
        // would start a conversation about nothing.
        if (!spoke && elapsed >= noSpeechMs) {
          end(false, "nothing said");
          return;
        }
        // The normal way a recording ends: the speaking stopped. The samples only go on when
        // a voice was actually heard — see `end`.
        if (longEnough || elapsed >= maxMs) {
          end(spoke, longEnough ? "pause" : "cap");
        }
      }, TICK_MS);
    });
  })();

  return {
    promise,
    // Stopping early — a second press, a click away — still hands over what was said, so the
    // sentence is not lost. A stop with no speech in it hands over nothing at all, which is
    // what keeps "press, say nothing, press" from costing three seconds of the engine.
    stop: () => end(spoke, "stopped"),
  };
}
