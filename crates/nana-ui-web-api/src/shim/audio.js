  function audioHostCall(name, args) {
    try {
      return hostCall(name, args);
    } catch (error) {
      const message = error && error.message ? String(error.message) : String(error || "audio operation failed");
      const errorName = error && error.name ? String(error.name) : "";
      if (errorName === "NotSupportedError" || errorName === "InvalidStateError" || errorName === "IndexSizeError") {
        throw new DOMException(message, errorName);
      }
      throw error;
    }
  }

  function audioUnsupported(name) {
    throw new DOMException(name + " is not implemented by the Nana Web Audio subset", "NotSupportedError");
  }

  function interleaveFloat32(channels) {
    if (!channels.length) return new Float32Array(0);
    const length = channels[0].length;
    const count = channels.length;
    const out = new Float32Array(length * count);
    for (let i = 0; i < length; i++) {
      for (let c = 0; c < count; c++) out[i * count + c] = channels[c][i] || 0;
    }
    return out;
  }

  function localAudioBuffer(channels, length, sampleRate) {
    const buffer = {
      numberOfChannels: channels,
      length: length,
      sampleRate: sampleRate,
      duration: length / sampleRate,
      _channels: [],
      getChannelData: function (channel) {
        const index = channel | 0;
        if (index < 0 || index >= this.numberOfChannels) {
          throw new DOMException("AudioBuffer channel is out of range", "IndexSizeError");
        }
        if (!this._channels[index]) this._channels[index] = new Float32Array(this.length);
        return this._channels[index];
      },
      copyToChannel: function (source, channel) {
        this.getChannelData(channel).set(source.subarray(0, this.length));
      },
      copyFromChannel: function (destination, channel) {
        destination.set(this.getChannelData(channel).subarray(0, destination.length));
      },
    };
    for (let c = 0; c < channels; c++) buffer._channels[c] = new Float32Array(length);
    return buffer;
  }

  function AudioParamShim(nodeId, initial) {
    this._id = nodeId;
    this._value = Number(initial);
    this.defaultValue = 1;
    this.minValue = 0;
    this.maxValue = 3.4028234663852886e+38;
  }
  Object.defineProperty(AudioParamShim.prototype, "value", {
    get: function () { return this._value; },
    set: function (value) {
      this._value = Number(value);
      audioHostCall("audioGainSetValue", [this._id, this._value]);
    },
  });

  function AudioNodeShim(resource) {
    EventTargetShim.call(this);
    this.__nanaAudioNode = resource;
    this.id = resource.id;
    this.context = resource.context;
    this.numberOfInputs = 0;
    this.numberOfOutputs = 1;
    this.channelCount = 2;
    this.channelCountMode = "max";
    this.channelInterpretation = "speakers";
  }
  AudioNodeShim.prototype = Object.create(EventTargetShim.prototype);
  AudioNodeShim.prototype.constructor = AudioNodeShim;
  AudioNodeShim.prototype.connect = function (destination) {
    const dest = destination && (destination.__nanaAudioNode || destination);
    const destId = dest && dest.id != null ? dest.id : destination && destination.id;
    if (destId == null) throw new TypeError("AudioNode.connect requires a Nana audio node");
    audioHostCall("audioNodeConnect", [this.id, destId]);
    this.__nanaConnected = true;
    return destination;
  };
  AudioNodeShim.prototype.disconnect = function () {};

  function AudioDestinationNodeShim(resource) {
    AudioNodeShim.call(this, resource);
    this.maxChannelCount = 2;
  }
  AudioDestinationNodeShim.prototype = Object.create(AudioNodeShim.prototype);
  AudioDestinationNodeShim.prototype.constructor = AudioDestinationNodeShim;

  function AudioBufferShim(resource) {
    this.__nanaAudioBuffer = resource;
    this.id = resource.id;
    this.numberOfChannels = Number(resource.numberOfChannels || 1);
    this.length = Number(resource.length || 0);
    this.sampleRate = Number(resource.sampleRate || 44100);
    this.duration = Number(resource.duration || 0);
    this._channels = [];
  }
  AudioBufferShim.prototype.getChannelData = function (channel) {
    const index = channel | 0;
    if (index < 0 || index >= this.numberOfChannels) {
      throw new DOMException("AudioBuffer channel is out of range", "IndexSizeError");
    }
    if (!this._channels[index]) {
      const bytes = audioHostCall("audioBufferGetChannelData", [this.id, index]);
      this._channels[index] = new Float32Array(asUint8Array(bytes).buffer);
    }
    return this._channels[index];
  };
  AudioBufferShim.prototype.copyToChannel = function (source, channel) {
    const data = source instanceof Float32Array ? source : Float32Array.from(source || []);
    this.getChannelData(channel).set(data.subarray(0, this.length));
    audioHostCall("audioBufferCopyToChannel", [this.id, channel | 0, this._channels[channel | 0]]);
  };
  AudioBufferShim.prototype.copyFromChannel = function (destination, channel) {
    destination.set(this.getChannelData(channel).subarray(0, destination.length));
  };
  AudioBufferShim.prototype.__nanaSyncChannels = function () {
    for (let channel = 0; channel < this.numberOfChannels; channel++) {
      if (this._channels[channel]) {
        audioHostCall("audioBufferCopyToChannel", [this.id, channel, this._channels[channel]]);
      }
    }
  };

  function AudioBufferSourceNodeShim(resource) {
    AudioNodeShim.call(this, resource);
    this.buffer = null;
    this._loop = false;
    this.playbackRate = { value: 1 };
    this.onended = null;
  }
  AudioBufferSourceNodeShim.prototype = Object.create(AudioNodeShim.prototype);
  AudioBufferSourceNodeShim.prototype.constructor = AudioBufferSourceNodeShim;
  // The mixer owns the loop flag, and WebAudio lets it be flipped mid-playback:
  // a plain field would keep the setting entirely on the JS side and every
  // source would play through exactly once.
  Object.defineProperty(AudioBufferSourceNodeShim.prototype, "loop", {
    get: function () { return this._loop; },
    set: function (value) {
      this._loop = !!value;
      audioHostCall("audioBufferSourceSetLoop", [this.id, this._loop]);
    },
  });
  AudioBufferSourceNodeShim.prototype.start = function () {
    if (this.buffer && typeof this.buffer.__nanaSyncChannels === "function") {
      this.buffer.__nanaSyncChannels();
      audioHostCall("audioBufferSourceSetBuffer", [this.id, this.buffer.id]);
    }
    audioHostCall("audioBufferSourceStart", [this.id]);
  };
  AudioBufferSourceNodeShim.prototype.stop = function () {
    audioHostCall("audioBufferSourceStop", [this.id]);
  };

  function GainNodeShim(resource) {
    AudioNodeShim.call(this, resource);
    this.gain = new AudioParamShim(this.id, 1);
  }
  GainNodeShim.prototype = Object.create(AudioNodeShim.prototype);
  GainNodeShim.prototype.constructor = GainNodeShim;

  // How far ahead of the wall clock the pump keeps the mixer, and the most it
  // will render in one tick after the timer was starved.
  const PUMP_LEAD_BLOCKS = 2;
  const PUMP_MAX_BLOCKS_PER_TICK = 8;

  function ScriptProcessorNodeShim(context, resource, bufferSize, inputChannels, outputChannels) {
    AudioNodeShim.call(this, resource);
    this.bufferSize = bufferSize;
    this.onaudioprocess = null;
    this._context = context;
    this._inputChannels = Math.max(1, inputChannels | 0);
    this._outputChannels = Math.max(1, outputChannels | 0);
    // One block per tick starves the mixer: a timer cannot tick faster than a
    // few milliseconds and always runs late, so a 256-frame block at 48 kHz
    // (5.3 ms of audio) was delivered every 10 ms or worse and half the output
    // came out silent. Tick on a timer, but produce by elapsed wall time and
    // keep a small lead, so jitter is absorbed instead of dropped.
    const blockMs = (bufferSize / Math.max(1, context.sampleRate)) * 1000;
    const interval = Math.max(4, Math.min(Math.round(blockMs), 50));
    const self = this;
    let submittedThrough = 0;
    this._pump = setInterval(function () {
      if (typeof self.onaudioprocess !== "function" || self._context.state !== "running") {
        submittedThrough = 0;
        return;
      }
      const now = Date.now();
      if (submittedThrough === 0) submittedThrough = now;
      const target = now + blockMs * PUMP_LEAD_BLOCKS;
      let blocks = Math.ceil((target - submittedThrough) / blockMs);
      if (blocks <= 0) return;
      if (blocks > PUMP_MAX_BLOCKS_PER_TICK) {
        // A stalled timer must not burst a backlog into the queue: skip ahead
        // rather than render audio the mixer has already played past.
        blocks = PUMP_MAX_BLOCKS_PER_TICK;
        submittedThrough = target - blocks * blockMs;
      }
      for (let i = 0; i < blocks; i++) {
        const inputBuffer = localAudioBuffer(self._inputChannels, self.bufferSize, self._context.sampleRate);
        const outputBuffer = localAudioBuffer(self._outputChannels, self.bufferSize, self._context.sampleRate);
        self.onaudioprocess({
          playbackTime: self._context.currentTime,
          inputBuffer: inputBuffer,
          outputBuffer: outputBuffer,
        });
        audioHostCall("audioScriptProcessorSubmit", [self.id, interleaveFloat32(outputBuffer._channels)]);
        submittedThrough += blockMs;
      }
    }, interval);
  }
  ScriptProcessorNodeShim.prototype = Object.create(AudioNodeShim.prototype);
  ScriptProcessorNodeShim.prototype.constructor = ScriptProcessorNodeShim;
  ScriptProcessorNodeShim.prototype.disconnect = function () {
    if (this._pump) {
      clearInterval(this._pump);
      this._pump = null;
    }
  };

  function AudioContextShim(options) {
    EventTargetShim.call(this);
    if (!globalThis.__nanaHost || typeof globalThis.__nanaHost.call !== "function") {
      throw new DOMException("no audio host or output device", "NotSupportedError");
    }
    let desc;
    try {
      desc = audioHostCall("audioContextCreate", [options && typeof options === "object" ? options : {}]);
    } catch (error) {
      const message = error && error.message ? String(error.message) : "no audio host or output device";
      throw new DOMException(message, "NotSupportedError");
    }
    if (!desc || desc.id == null) {
      throw new DOMException("no audio host or output device", "NotSupportedError");
    }
    this.__nanaAudioContext = desc;
    this.id = desc.id;
    this.sampleRate = Number(desc.sampleRate || 44100);
    this.state = desc.state || "running";
    this.destination = new AudioDestinationNodeShim(desc.destination);
    this.baseLatency = 0;
    this.outputLatency = 0;
    this.audioWorklet = {
      addModule: function () {
        return Promise.reject(new DOMException("AudioWorklet is not implemented by the Nana Web Audio subset", "NotSupportedError"));
      },
    };
  }
  AudioContextShim.prototype = Object.create(EventTargetShim.prototype);
  AudioContextShim.prototype.constructor = AudioContextShim;
  Object.defineProperty(AudioContextShim.prototype, "currentTime", {
    get: function () {
      try { return Number(audioHostCall("audioContextCurrentTime", [this.id]) || 0); }
      catch (_err) { return 0; }
    },
  });
  AudioContextShim.prototype.createBuffer = function (numberOfChannels, length, sampleRate) {
    return new AudioBufferShim(audioHostCall("audioBufferCreate", [this.id, numberOfChannels, length, sampleRate]));
  };
  AudioContextShim.prototype.createBufferSource = function () {
    return new AudioBufferSourceNodeShim(audioHostCall("audioBufferSourceCreate", [this.id]));
  };
  AudioContextShim.prototype.createGain = function () {
    return new GainNodeShim(audioHostCall("audioGainCreate", [this.id]));
  };
  AudioContextShim.prototype.createScriptProcessor = function (bufferSize, inputChannels, outputChannels) {
    const size = bufferSize || 4096;
    const inputs = inputChannels == null ? 2 : inputChannels;
    const outputs = outputChannels == null ? 2 : outputChannels;
    const resource = audioHostCall("audioScriptProcessorCreate", [this.id, size, inputs, outputs]);
    return new ScriptProcessorNodeShim(this, resource, size, inputs, outputs);
  };
  AudioContextShim.prototype.decodeAudioData = function () {
    return Promise.reject(new DOMException("decodeAudioData is not implemented by the Nana Web Audio subset", "NotSupportedError"));
  };
  AudioContextShim.prototype.resume = function () {
    const self = this;
    return Promise.resolve().then(function () {
      audioHostCall("audioContextResume", [self.id]);
      self.state = "running";
    });
  };
  AudioContextShim.prototype.suspend = function () {
    const self = this;
    return Promise.resolve().then(function () {
      audioHostCall("audioContextSuspend", [self.id]);
      self.state = "suspended";
    });
  };
  AudioContextShim.prototype.close = function () {
    const self = this;
    return Promise.resolve().then(function () {
      audioHostCall("audioContextClose", [self.id]);
      self.state = "closed";
    });
  };
  [
    "createAnalyser", "createBiquadFilter", "createOscillator", "createPanner",
    "createStereoPanner", "createDelay", "createConvolver", "createDynamicsCompressor",
    "createWaveShaper", "createPeriodicWave", "createChannelSplitter", "createChannelMerger",
    "createMediaElementSource", "createMediaStreamSource", "createMediaStreamDestination",
    "createConstantSource", "createIIRFilter", "createAudioWorkletNode",
  ].forEach(function (name) {
    AudioContextShim.prototype[name] = function () { audioUnsupported(name); };
  });
