//! Audio and video, the same way as photos: ffmpeg decodes the untrusted file only inside the
//! sandbox and hands back raw samples or frames; here, a fresh file is encoded from those alone.
//! The ffmpeg run here never parses the original: its only inputs are raw PCM and raw yuv420p
//! frames at a size we chose.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Photo, ReviewError, read_capped, read_untrusted, sandbox};

/// What the sandbox says is in a file. Every number is clamped; nothing else is read.
#[derive(Debug, Clone, Copy, Default)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
}

/// A voice note or audio file rebuilt from its samples.
#[derive(Debug)]
pub struct Audio {
    pub duration: f64,
    /// 64 bars, 0 to 255, for the waveform.
    pub peaks: Vec<u8>,
    /// AAC in an MP4 container (`audio/mp4`).
    pub bytes: Vec<u8>,
}

const VOICE_RATE: u32 = 24_000;
const VIDEO_RATE: u32 = 48_000;
/// Longest side of a playable video.
pub const VIDEO_MAX: u32 = 1280;

fn tool_missing() -> ReviewError {
    ReviewError::Render("audio and video need ffmpeg, which isn't installed on this computer".into())
}

fn sandboxed(render: &Path, input: &Path, out: &Path, timeout: Duration, mode: &str) -> Result<(), ReviewError> {
    fs::create_dir_all(out).map_err(|_| ReviewError::Io)?;
    sandbox::run_mode(render, input, out, timeout, Some(mode)).map(|_| ())
}

pub fn probe(input: &Path, work: &Path, render: &Path, timeout: Duration) -> Result<Probe, ReviewError> {
    let out = work.join("probe");
    sandboxed(render, input, &out, timeout, "probe")?;
    let text = read_capped(&out.join("info.txt"), 16 * 1024);
    let _ = fs::remove_dir_all(&out);
    let mut p = Probe::default();
    let mut in_video = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "codec_type" => {
                in_video = v.trim() == "video";
                p.has_video |= in_video;
                p.has_audio |= v.trim() == "audio";
            }
            "width" if in_video && p.width == 0 => p.width = v.trim().parse::<u32>().unwrap_or(0).min(16_384),
            "height" if in_video && p.height == 0 => p.height = v.trim().parse::<u32>().unwrap_or(0).min(16_384),
            "duration" => {
                if let Ok(d) = v.trim().parse::<f64>()
                    && d.is_finite()
                    && d > 0.0
                {
                    p.duration = d.min(24.0 * 3600.0);
                }
            }
            _ => {}
        }
    }
    if !p.has_video && !p.has_audio {
        return Err(ReviewError::Render("this isn't a video or audio file Sotto can open".into()));
    }
    Ok(p)
}

/// Raw mono samples from the sandbox, at `rate`.
fn samples(input: &Path, work: &Path, render: &Path, timeout: Duration, rate: u32) -> Result<Vec<u8>, ReviewError> {
    let out = work.join(format!("audio-{rate}"));
    sandboxed(render, input, &out, timeout, &format!("audio:{rate}"))?;
    let cap = rate as usize * 2 * 1800 + 16;
    let pcm = read_untrusted(&out.join("audio.pcm"), cap).ok_or_else(|| ReviewError::Render("unexpected output from the renderer".into()))?;
    let _ = fs::remove_dir_all(&out);
    if pcm.len() < 2 {
        return Err(ReviewError::Render("there's no sound in this file".into()));
    }
    Ok(pcm)
}

/// 64 loudness bars from 16-bit samples, scaled so the loudest is 255.
pub fn peaks(pcm: &[u8]) -> Vec<u8> {
    let n = pcm.len() / 2;
    let bars = 64;
    if n == 0 {
        return vec![0; bars];
    }
    let mut rms = vec![0f64; bars];
    for (b, slot) in rms.iter_mut().enumerate() {
        let from = b * n / bars;
        let to = ((b + 1) * n / bars).max(from + 1).min(n);
        let mut sum = 0f64;
        for i in from..to {
            let v = i16::from_le_bytes([pcm[2 * i], pcm[2 * i + 1]]) as f64 / 32768.0;
            sum += v * v;
        }
        *slot = (sum / (to - from) as f64).sqrt();
    }
    let max = rms.iter().cloned().fold(0f64, f64::max).max(1e-4);
    rms.iter().map(|v| ((v / max).sqrt() * 255.0).round() as u8).collect()
}

fn wait(child: &mut std::process::Child, timeout: Duration) -> Result<(), ReviewError> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(_)) => return Err(ReviewError::Render("the clean copy couldn't be made".into())),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ReviewError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err(ReviewError::Io),
        }
    }
}

/// Start ffmpeg here, on raw input only.
fn ffmpeg(args: &[String]) -> Result<std::process::Child, ReviewError> {
    Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| tool_missing())
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A voice note or audio file: samples from the sandbox, a waveform, and fresh AAC.
pub fn audio(input: &Path, work: &Path, render: &Path, timeout: Duration) -> Result<Audio, ReviewError> {
    let pcm = samples(input, work, render, timeout, VOICE_RATE)?;
    let duration = (pcm.len() / 2) as f64 / VOICE_RATE as f64;
    let peaks = peaks(&pcm);
    let raw = work.join("voice.pcm");
    let out = work.join("voice.m4a");
    fs::write(&raw, &pcm).map_err(|_| ReviewError::Io)?;
    drop(pcm);
    let mut args = strs(&["-f", "s16le", "-ar", &VOICE_RATE.to_string(), "-ac", "1", "-i"]);
    args.push(raw.display().to_string());
    args.extend(strs(&["-c:a", "aac", "-b:a", "64k", "-movflags", "+faststart", "-f", "mp4"]));
    args.push(out.display().to_string());
    wait(&mut ffmpeg(&args)?, timeout)?;
    let bytes = fs::read(&out).map_err(|_| ReviewError::Io)?;
    let _ = fs::remove_file(&raw);
    let _ = fs::remove_file(&out);
    Ok(Audio { duration, peaks, bytes })
}

/// A video's first frame, as a clean photo.
pub fn poster(input: &Path, work: &Path, render: &Path, timeout: Duration) -> Result<Photo, ReviewError> {
    let out = work.join("poster");
    sandboxed(render, input, &out, timeout, "poster")?;
    let photo = crate::photo_from(&out);
    let _ = fs::remove_dir_all(&out);
    photo
}

/// The size a video plays at: longest side at most 1280, both sides even.
pub fn play_size(p: &Probe) -> (u32, u32) {
    let (w, h) = (p.width.max(16) as f64, p.height.max(16) as f64);
    let scale = (VIDEO_MAX as f64 / w.max(h)).min(1.0);
    let even = |v: f64| (((v * scale) / 2.0).round() as u32 * 2).clamp(16, VIDEO_MAX);
    (even(w), even(h))
}

/// A video, playable: frames stream out of the sandbox as raw yuv420p through a pipe, and are
/// encoded here (H.264 and AAC in MP4) together with the raw audio.
pub fn video(input: &Path, work: &Path, render: &Path, p: &Probe, timeout: Duration) -> Result<Vec<u8>, ReviewError> {
    if !p.has_video || p.width == 0 || p.height == 0 {
        return Err(ReviewError::Render("there's no picture in this video".into()));
    }
    let (w, h) = play_size(p);
    let audio = if p.has_audio {
        let pcm = samples(input, work, render, timeout, VIDEO_RATE)?;
        let path = work.join("video-audio.pcm");
        fs::write(&path, pcm).map_err(|_| ReviewError::Io)?;
        Some(path)
    } else {
        None
    };
    let raw = work.join("frames");
    fs::create_dir_all(&raw).map_err(|_| ReviewError::Io)?;
    let fifo = raw.join("video.yuv");
    if !Command::new("mkfifo").arg(&fifo).status().map(|s| s.success()).unwrap_or(false) {
        return Err(ReviewError::Io);
    }
    let out = work.join("play.mp4");
    let mut args = strs(&["-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{w}x{h}"), "-r", "30", "-i"]);
    args.push(fifo.display().to_string());
    if let Some(a) = &audio {
        args.extend(strs(&["-f", "s16le", "-ar", &VIDEO_RATE.to_string(), "-ac", "1", "-i"]));
        args.push(a.display().to_string());
        args.extend(strs(&["-map", "0:v", "-map", "1:a", "-c:a", "aac", "-b:a", "96k", "-shortest"]));
    }
    args.extend(strs(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-profile:v", "main", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-f", "mp4"]));
    args.push(out.display().to_string());
    // The encoder opens the pipe and waits; the sandbox then writes frames into it.
    let mut encoder = ffmpeg(&args)?;
    let decoded = sandbox::run_mode(render, input, &raw, timeout, Some(&format!("video:{w}x{h}")));
    if let Err(e) = decoded {
        // Unblock an encoder still waiting for the pipe to open, then stop it.
        let _ = fs::OpenOptions::new().write(true).open(&fifo).map(drop);
        let _ = encoder.kill();
        let _ = encoder.wait();
        return Err(e);
    }
    wait(&mut encoder, timeout)?;
    let mut bytes = Vec::new();
    fs::File::open(&out).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|_| ReviewError::Io)?;
    let _ = fs::remove_dir_all(&raw);
    let _ = fs::remove_file(&out);
    if let Some(a) = audio {
        let _ = fs::remove_file(a);
    }
    Ok(bytes)
}

/// Your own video for sending: same streams, every metadata tag dropped. `ext` is the
/// container to keep (mp4 for MP4 and QuickTime, webm, or mkv).
pub fn remux(input: &Path, work: &Path, render: &Path, ext: &str, timeout: Duration) -> Result<Vec<u8>, ReviewError> {
    let out = work.join("remux");
    sandboxed(render, input, &out, timeout, &format!("remux:{ext}"))?;
    let bytes = read_untrusted(&out.join(format!("clean.{ext}")), 1 << 30).ok_or_else(|| ReviewError::Render("unexpected output from the renderer".into()))?;
    let _ = fs::remove_dir_all(&out);
    Ok(bytes)
}
