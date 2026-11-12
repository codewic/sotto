// Run with: bun test src/call/sdp.test.ts
import { expect, test } from "bun:test";
import { constantBitrateOpus, privateSdp, stripAudioLevel } from "./sdp";

const CHROME_AUDIO = [
  "v=0",
  "a=group:BUNDLE 0 1",
  "m=audio 9 UDP/TLS/RTP/SAVPF 111 63",
  "a=mid:0",
  "a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level",
  "a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid",
  "a=rtpmap:111 opus/48000/2",
  "a=fmtp:111 minptime=10;useinbandfec=1;usedtx=1",
  "a=rtpmap:63 red/48000/2",
  "a=fmtp:63 111/111",
  "m=audio 9 UDP/TLS/RTP/SAVPF 111",
  "a=mid:1",
  "a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level",
  "a=rtpmap:111 opus/48000/2",
  "",
].join("\r\n");

test("the audio-level extension is never offered", () => {
  const out = stripAudioLevel(CHROME_AUDIO);
  expect(out).not.toContain("ssrc-audio-level");
  expect(out).toContain("a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid");
  expect(out.endsWith("\r\n")).toBe(true);
});

test("every opus payload asks for constant bitrate and no DTX, alike across sections", () => {
  const out = constantBitrateOpus(CHROME_AUDIO);
  expect(out).toContain("a=fmtp:111 minptime=10;useinbandfec=1;cbr=1;usedtx=0");
  expect(out).not.toContain("usedtx=1");
  // The second section had no fmtp line: it gets one with the same values.
  const fmtps = out.split("\r\n").filter((l) => l.startsWith("a=fmtp:111"));
  expect(fmtps.length).toBe(2);
  expect(fmtps[1]).toBe("a=fmtp:111 cbr=1;usedtx=0");
  // Other codecs are left alone.
  expect(out).toContain("a=fmtp:63 111/111");
});

test("rewriting twice changes nothing more", () => {
  const once = privateSdp(CHROME_AUDIO);
  expect(privateSdp(once)).toBe(once);
  expect(once.match(/cbr=1/g)?.length).toBe(2);
});

test("descriptions without opus pass through", () => {
  const video = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n";
  expect(privateSdp(video)).toBe(video);
});
