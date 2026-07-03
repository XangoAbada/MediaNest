//! Miniaturki, sprite'y scrubbingu i metadane wideo przez ffmpeg-sidecar.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::event::FfmpegEvent;

pub static FFMPEG_READY: AtomicBool = AtomicBool::new(false);
static HAS_CUDA: AtomicBool = AtomicBool::new(false);

pub fn ready() -> bool {
    FFMPEG_READY.load(Ordering::Relaxed)
}

/// Pobiera ffmpeg (jeśli brak) i wykrywa akcelerację NVIDIA. Wołane raz, w tle.
/// ponytail: auto_download obok exe wystarcza w dev; instalator w Fazie 7
/// zbunduje ffmpeg jako zasób Tauri.
pub fn init() {
    std::env::set_var("KEEP_ONLY_FFMPEG", "1");
    if let Err(e) = ffmpeg_sidecar::download::auto_download() {
        eprintln!("ffmpeg download failed: {e} — wideo pozostaną w kolejce");
        return;
    }
    if let Ok(path) = ffmpeg_sidecar::paths::ffmpeg_path().into_os_string().into_string() {
        if let Ok(out) = std::process::Command::new(&path).arg("-hwaccels").output() {
            let accels = String::from_utf8_lossy(&out.stdout).to_lowercase();
            HAS_CUDA.store(accels.contains("cuda"), Ordering::Relaxed);
        }
    }
    FFMPEG_READY.store(true, Ordering::Relaxed);
    println!(
        "ffmpeg gotowy (cuda: {})",
        HAS_CUDA.load(Ordering::Relaxed)
    );
}

pub struct VideoMeta {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Czyta nagłówek pliku: czas trwania i wymiary. Dekoduje tylko 1 klatkę.
pub fn probe(path: &Path) -> anyhow::Result<VideoMeta> {
    let mut meta = VideoMeta {
        duration: None,
        width: None,
        height: None,
    };
    let mut child = FfmpegCommand::new()
        .input(path.to_string_lossy())
        .args(["-frames:v", "1", "-f", "null", "-"])
        .spawn()?;
    for event in child.iter()? {
        match event {
            FfmpegEvent::ParsedDuration(d) => meta.duration = Some(d.duration),
            FfmpegEvent::ParsedInputStream(stream) => {
                if let Some(v) = stream.video_data() {
                    meta.width = Some(v.width);
                    meta.height = Some(v.height);
                }
            }
            _ => {}
        }
    }
    Ok(meta)
}

fn run_to_file(build: impl Fn(&mut FfmpegCommand), out: &Path) -> bool {
    let cuda = HAS_CUDA.load(Ordering::Relaxed);
    // próba z NVDEC, przy niepowodzeniu fallback na CPU
    for use_cuda in [cuda, false] {
        let mut cmd = FfmpegCommand::new();
        if use_cuda {
            cmd.args(["-hwaccel", "cuda"]);
        }
        build(&mut cmd);
        cmd.args(["-y"]).output(out.to_string_lossy());
        if let Ok(mut child) = cmd.spawn() {
            if let Ok(iter) = child.iter() {
                iter.for_each(drop);
            }
        }
        if out.exists() {
            return true;
        }
        if !use_cuda {
            break;
        }
    }
    false
}

/// Miniaturka z klatki ~10% czasu trwania.
pub fn thumbnail(path: &Path, duration: Option<f64>, out: &Path) -> bool {
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    let seek = duration.map(|d| d * 0.1).unwrap_or(0.0);
    let input = path.to_string_lossy().to_string();
    run_to_file(
        |cmd| {
            cmd.args(["-ss", &format!("{seek:.2}")])
                .input(&input)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=256:256:force_original_aspect_ratio=decrease",
                ]);
        },
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_thumbnail_sprite_roundtrip() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("pominięto: brak ffmpeg w PATH");
            return;
        }
        let dir = std::env::temp_dir().join("medianest-test-video");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let vid = dir.join("test.mp4");
        assert!(std::process::Command::new("ffmpeg")
            .args(["-f", "lavfi", "-i", "testsrc2=duration=5:size=320x240:rate=10"])
            .args(["-pix_fmt", "yuv420p", "-y"])
            .arg(&vid)
            .output()
            .unwrap()
            .status
            .success());

        let meta = probe(&vid).unwrap();
        assert_eq!(meta.width, Some(320));
        assert_eq!(meta.height, Some(240));
        let dur = meta.duration.expect("duration sparsowane");
        assert!((dur - 5.0).abs() < 0.5, "dur={dur}");

        let thumb = dir.join("t.webp");
        assert!(thumbnail(&vid, Some(dur), &thumb));
        let img = image::open(&thumb).unwrap();
        assert!(img.width() <= 256 && img.height() <= 256);

        let spr = dir.join("s.webp");
        assert!(sprite(&vid, dur, &spr));
        let img = image::open(&spr).unwrap();
        assert_eq!(img.width(), 1600); // 10 klatek × 160 px
    }
}

/// Sprite 10 klatek (10x1) do scrubbingu na hover.
pub fn sprite(path: &Path, duration: f64, out: &Path) -> bool {
    if duration < 2.0 {
        return false; // za krótkie na sensowny scrubbing
    }
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    let input = path.to_string_lossy().to_string();
    // fps=11/dur daje zapas — tile=10x1 nie wyemituje klatki, jeśli zaokrąglenie
    // fps zbierze tylko 9 klatek; nadmiarową ucina -frames:v 1
    let vf = format!("fps=11/{duration:.3},scale=160:-2,tile=10x1");
    run_to_file(
        |cmd| {
            cmd.input(&input).args(["-vf", &vf, "-frames:v", "1"]);
        },
        out,
    )
}
