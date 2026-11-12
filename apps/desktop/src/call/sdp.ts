// Keeps the call server from learning when people speak.
//
// Media is end-to-end encrypted, but two things around it would still reveal speech:
//  - the audio-level RTP header extension, which carries each packet's loudness in the clear
//    (servers use it for "who's speaking"); and
//  - Opus at a variable bitrate, where packet sizes rise and fall with speech.
// Every session description this window sets passes through privateSdp(): the audio-level
// extension is never negotiated, and Opus is asked for a constant bitrate with no
// discontinuous transmission. "Who's speaking" is worked out on the device from decrypted audio.

const AUDIO_LEVEL = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";

function split(sdp: string): { lines: string[]; eol: string } {
  const eol = sdp.includes("\r\n") ? "\r\n" : "\n";
  return { lines: sdp.split(eol), eol };
}

/** Never negotiate the audio-level header extension. */
export function stripAudioLevel(sdp: string): string {
  const { lines, eol } = split(sdp);
  return lines.filter((l) => !(l.startsWith("a=extmap:") && l.includes(AUDIO_LEVEL))).join(eol);
}

/**
 * Constant bitrate and no discontinuous transmission on every Opus payload type. Applied to every
 * section alike: within a bundle a payload type must carry identical parameters everywhere.
 */
export function constantBitrateOpus(sdp: string): string {
  const { lines, eol } = split(sdp);
  const opus = new Set<string>();
  for (const l of lines) {
    const m = /^a=rtpmap:(\d+) opus\/48000/i.exec(l);
    if (m) opus.add(m[1]);
  }
  if (opus.size === 0) return sdp;
  // fmtp lines belong to their media section, so work section by section.
  const sections: string[][] = [[]];
  for (const l of lines) {
    if (l.startsWith("m=")) sections.push([]);
    sections[sections.length - 1].push(l);
  }
  const result: string[] = [];
  for (const section of sections) {
    const withFmtp = new Set<string>();
    const out: string[] = [];
    for (const l of section) {
      const f = /^a=fmtp:(\d+) (.*)$/.exec(l);
      if (f && opus.has(f[1])) {
        withFmtp.add(f[1]);
        const params = f[2]
          .split(";")
          .map((p) => p.trim())
          .filter((p) => p && !/^(cbr|usedtx)=/i.test(p));
        out.push(`a=fmtp:${f[1]} ${[...params, "cbr=1", "usedtx=0"].join(";")}`);
        continue;
      }
      out.push(l);
    }
    // An Opus payload type with no fmtp line in this section gets one right after its rtpmap.
    for (const l of out) {
      result.push(l);
      const m = /^a=rtpmap:(\d+) opus\/48000/i.exec(l);
      if (m && !withFmtp.has(m[1])) result.push(`a=fmtp:${m[1]} cbr=1;usedtx=0`);
    }
  }
  return result.join(eol);
}

export function privateSdp(sdp: string): string {
  return constantBitrateOpus(stripAudioLevel(sdp));
}

/** Rewrite every session description this window sets, local and remote. Idempotent. */
export function installSdpPrivacy() {
  const proto = RTCPeerConnection.prototype as unknown as Record<string, unknown> & { __sottoPrivacy?: boolean };
  if (proto.__sottoPrivacy) return;
  proto.__sottoPrivacy = true;
  for (const name of ["setLocalDescription", "setRemoteDescription"]) {
    const original = proto[name] as (this: RTCPeerConnection, d?: RTCSessionDescriptionInit, ...rest: unknown[]) => Promise<void>;
    proto[name] = function (this: RTCPeerConnection, d?: RTCSessionDescriptionInit, ...rest: unknown[]) {
      const desc = d && typeof d.sdp === "string" && d.sdp ? { type: d.type, sdp: privateSdp(d.sdp) } : d;
      return original.call(this, desc, ...rest);
    };
  }
}
