//! Screencast frames to disk, then (when ffmpeg is on PATH) to a video file.
//!
//! Chrome only emits a frame when the page repaints, so each frame keeps its
//! own timestamp and the ffconcat list gives it an exact on-screen duration.
//! A static page costs one frame however long it sits there.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Output frame rate. It matches the image demuxer's 1/25 timebase, so the
/// grid-aligned durations below survive ffmpeg without rounding drift.
const FPS: f64 = 25.0;

pub struct Recorder {
    out: String,
    dir: PathBuf,
    /// Frame times on the video's clock, which `speed` runs faster or slower.
    stamps: Vec<f64>,
    /// Since `anchor.0` (real time), the video's clock runs at `speed`.
    anchor: (f64, f64),
    speed: f64,
    /// First frame that could not be stored; `finish` reports it.
    error: Option<String>,
}

/// How a recording ended: the file or frames directory, and whether it fell
/// short of a video (no ffmpeg, or an extension ffmpeg was not asked to make).
pub struct Finished {
    pub path: String,
    pub encoded: bool,
}

impl Recorder {
    pub fn start(out: &str) -> Result<Self, String> {
        let dir = PathBuf::from(format!("{out}.frames"));
        // Frames from an earlier run would otherwise mix into this one.
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create '{}': {e}", dir.display()))?;
        Ok(Self {
            out: out.to_string(),
            dir,
            stamps: Vec::new(),
            anchor: (0.0, 0.0),
            speed: 1.0,
            error: None,
        })
    }

    /// Real time `t` on the video's clock.
    fn at(&self, t: f64) -> f64 {
        self.anchor.1 + (t - self.anchor.0) / self.speed
    }

    /// From real time `at` on, play the video at `factor` times real speed.
    pub fn set_speed(&mut self, at: f64, factor: f64) {
        self.anchor = (at, self.at(at));
        self.speed = factor;
    }

    pub fn set_speed_now(&mut self, factor: f64) {
        self.set_speed(now_secs(), factor);
    }

    /// Store a frame. A failure is kept for `finish` rather than returned,
    /// because frames arrive inside unrelated commands that must not fail.
    pub fn add(&mut self, timestamp: f64, jpeg: Result<Vec<u8>, String>) {
        let path = self.dir.join(frame_name(self.stamps.len()));
        let stored = jpeg.and_then(|jpeg| {
            fs::write(&path, jpeg).map_err(|e| format!("failed to write '{}': {e}", path.display()))
        });
        match stored {
            Ok(()) => self.stamps.push(self.at(timestamp)),
            Err(e) => {
                self.error.get_or_insert(e);
            }
        }
    }

    /// Drop a recording that never started.
    pub fn discard(self) {
        let _ = fs::remove_dir_all(&self.dir);
    }

    pub fn finish(self) -> Result<Finished, String> {
        if let Some(e) = self.error {
            return Err(format!(
                "recording dropped a frame (frames kept in '{}'): {e}",
                self.dir.display()
            ));
        }
        if self.stamps.is_empty() {
            let _ = fs::remove_dir_all(&self.dir);
            return Err("recording captured no frames".to_string());
        }
        let list = self.dir.join("frames.ffconcat");
        let end = self.at(now_secs());
        fs::write(&list, concat_list(&self.stamps, end))
            .map_err(|e| format!("failed to write '{}': {e}", list.display()))?;

        let frames = Finished {
            path: self.dir.display().to_string(),
            encoded: false,
        };
        let Some(ext) = video_ext(&self.out) else {
            return Ok(frames);
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
        ])
        .arg(&list)
        // ffmpeg gives the final list entry the previous entry's duration, so
        // trim to the true length. The fps filter makes CFR on any ffmpeg version.
        .args(["-t", &format!("{:.2}", grid_len(&self.stamps, end))]);
        if matches!(ext, "mp4" | "mov") {
            // x264 needs even dimensions; yuv420p keeps the file playable everywhere.
            cmd.args([
                "-vf",
                &format!("fps={FPS},pad=ceil(iw/2)*2:ceil(ih/2)*2"),
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ]);
        } else {
            cmd.args(["-vf", &format!("fps={FPS}")]);
        }
        cmd.arg(&self.out);

        match cmd.output() {
            Ok(o) if o.status.success() => {
                let _ = fs::remove_dir_all(&self.dir);
                Ok(Finished {
                    path: self.out,
                    encoded: true,
                })
            }
            Ok(o) => Err(format!(
                "ffmpeg failed (frames kept in '{}'): {}",
                self.dir.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(frames),
            Err(e) => Err(format!("failed to run ffmpeg: {e}")),
        }
    }
}

fn frame_name(i: usize) -> String {
    format!("{i:06}.jpg")
}

fn video_ext(path: &str) -> Option<&str> {
    let ext = path.rsplit_once('.')?.1;
    matches!(ext, "mp4" | "mov" | "webm" | "gif" | "mkv").then_some(ext)
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn ticker(stamps: &[f64], end: f64) -> impl Fn(f64) -> i64 {
    let t0 = stamps.first().copied().unwrap_or(end);
    move |t| ((t - t0) * FPS).round() as i64
}

/// Seconds the list covers: first frame to `end`, on the FPS grid.
fn grid_len(stamps: &[f64], end: f64) -> f64 {
    let tick = ticker(stamps, end);
    // Matches `concat_list`, which always gives the last frame a tick.
    let last = stamps.last().map_or(0, |t| tick(*t) + 1);
    tick(end).max(last) as f64 / FPS
}

/// Each frame shows until the next one, the last until `end`. Times snap to
/// the FPS grid from absolute time (no drift), and a frame that owns no tick
/// is dropped. The trailing repeat makes ffmpeg show the last frame at all.
fn concat_list(stamps: &[f64], end: f64) -> String {
    let tick = ticker(stamps, end);
    let mut s = String::from("ffconcat version 1.0\n");
    let mut last = None;
    for (i, t) in stamps.iter().enumerate() {
        let next = stamps.get(i + 1).copied().unwrap_or(end);
        let mut ticks = tick(next) - tick(*t);
        if i + 1 == stamps.len() {
            // No later frame replaces the last one, so it always gets a tick.
            ticks = ticks.max(1);
        }
        if ticks > 0 {
            s.push_str(&format!(
                "file '{}'\nduration {:.2}\n",
                frame_name(i),
                ticks as f64 / FPS
            ));
            last = Some(i);
        }
    }
    if let Some(i) = last {
        s.push_str(&format!("file '{}'\n", frame_name(i)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reports_a_frame_that_failed_to_store() {
        let out = std::env::temp_dir().join(format!("wd-rec-err-{}.mp4", std::process::id()));
        let mut rec = Recorder::start(out.to_str().unwrap()).unwrap();
        rec.add(1.0, Ok(vec![0xff]));
        rec.add(2.0, Err("bad screencast frame: x".to_string()));
        rec.add(3.0, Ok(vec![0xff]));
        let err = rec.finish().err().expect("finish must fail");
        assert!(err.contains("bad screencast frame: x"), "{err}");
        let _ = fs::remove_dir_all(format!("{}.frames", out.display()));
    }

    #[test]
    fn concat_list_times_each_frame_until_the_next() {
        // 10.0 and 10.01 share tick 0; the later frame wins. Sum = end - start.
        let list = concat_list(&[10.0, 10.01, 10.5, 12.0], 13.25);
        assert_eq!(
            list,
            "ffconcat version 1.0\n\
             file '000001.jpg'\nduration 0.52\n\
             file '000002.jpg'\nduration 1.48\n\
             file '000003.jpg'\nduration 1.24\n\
             file '000003.jpg'\n"
        );
    }

    #[test]
    fn last_frame_survives_a_stop_right_after_it() {
        let list = concat_list(&[10.0, 11.0], 11.01);
        assert!(list.ends_with("file '000001.jpg'\nduration 0.04\nfile '000001.jpg'\n"));
        assert_eq!(grid_len(&[10.0, 11.0], 11.01), 1.04);
    }

    #[test]
    fn speed_shortens_only_what_follows_it() {
        let out = std::env::temp_dir().join(format!("wd-rec-speed-{}.mp4", std::process::id()));
        let mut rec = Recorder::start(out.to_str().unwrap()).unwrap();
        rec.add(10.0, Ok(vec![0xff]));
        rec.add(11.0, Ok(vec![0xff]));
        rec.set_speed(12.0, 4.0);
        rec.add(12.0, Ok(vec![0xff]));
        rec.add(16.0, Ok(vec![0xff]));
        // 2s at 1x, then 8s at 4x: the video is 4s, not 10s.
        assert_eq!(rec.stamps, vec![10.0, 11.0, 12.0, 13.0]);
        assert_eq!(rec.at(20.0), 14.0);
        rec.discard();
    }

    #[test]
    fn grid_len_matches_the_listed_durations() {
        assert_eq!(grid_len(&[10.0, 10.01, 10.5, 12.0], 13.25), 3.24);
    }

    #[test]
    fn only_video_extensions_encode() {
        assert_eq!(video_ext("demo.mp4"), Some("mp4"));
        assert_eq!(video_ext("out/demo.gif"), Some("gif"));
        assert_eq!(video_ext("demo"), None);
        assert_eq!(video_ext("demo.png"), None);
    }
}
