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
    /// Kodek pierwszego strumienia wideo, np. `h264`, `hevc` — do decyzji, czy
    /// WebView2 odtworzy plik bezpośrednio.
    pub codec: Option<String>,
}

/// Czyta nagłówek pliku: czas trwania, wymiary i kodek. Dekoduje tylko 1 klatkę.
pub fn probe(path: &Path) -> anyhow::Result<VideoMeta> {
    let mut meta = VideoMeta {
        duration: None,
        width: None,
        height: None,
        codec: None,
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
                    if meta.codec.is_none() {
                        meta.codec = Some(stream.format.clone());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(meta)
}

/// Czy WebView2 odtworzy plik bezpośrednio: kontener mp4/mov/m4v z kodekiem
/// H.264. Wszystko inne (AVI/WMV/MTS/MKV/3GP, HEVC) wymaga transkodowania.
pub fn web_playable(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "mp4" | "m4v" | "mov") {
        return false;
    }
    matches!(
        probe(path).ok().and_then(|m| m.codec).as_deref(),
        Some("h264")
    )
}

/// Parsuje `HH:MM:SS.ms` z postępu ffmpeg na sekundy.
fn parse_hms(t: &str) -> Option<f64> {
    let mut parts = t.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Transkoduje do H.264/AAC MP4 z faststart (moov na początku → szybki start
/// i seek przez Range). Próba NVENC, fallback na libx264. `on_progress` dostaje
/// postęp 0.0–1.0 wyliczony z czasu przetworzonego / czasu trwania.
pub fn transcode_web(src: &Path, out: &Path, duration: Option<f64>, on_progress: impl Fn(f64)) -> bool {
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    let cuda = HAS_CUDA.load(Ordering::Relaxed);
    let input = src.to_string_lossy().to_string();
    let tmp = out.with_extension("mp4.part");
    for use_cuda in [cuda, false] {
        let mut cmd = FfmpegCommand::new();
        if use_cuda {
            cmd.args(["-hwaccel", "cuda"]);
        }
        cmd.input(&input);
        if use_cuda {
            cmd.args(["-c:v", "h264_nvenc"]);
        } else {
            cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"]);
        }
        // -f mp4: nazwa tmp (.part) nie wskazuje muxera, wymuszamy jawnie
        cmd.args(["-c:a", "aac", "-movflags", "+faststart", "-f", "mp4", "-y"])
            .output(tmp.to_string_lossy());
        let mut success = false;
        if let Ok(mut child) = cmd.spawn() {
            if let Ok(iter) = child.iter() {
                for event in iter {
                    if let FfmpegEvent::Progress(p) = event {
                        if let (Some(d), Some(t)) = (duration, parse_hms(&p.time)) {
                            if d > 0.0 {
                                on_progress((t / d).clamp(0.0, 1.0));
                            }
                        }
                    }
                }
            }
            success = child.wait().map(|s| s.success()).unwrap_or(false);
        }
        // akceptuj tylko przy sukcesie ffmpeg i niepustym pliku — NVENC/NVDEC
        // potrafi zwrócić błąd zostawiając pusty plik; wtedy fallback na CPU
        let nonempty = tmp.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if success && nonempty {
            // atomowy commit: częściowy plik nigdy nie trafia do cache jako gotowy
            if std::fs::rename(&tmp, out).is_ok() {
                on_progress(1.0);
                return true;
            }
        }
        std::fs::remove_file(&tmp).ok(); // sprzątaj przed fallbackiem / wyjściem
        if !use_cuda {
            break;
        }
    }
    false
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

        // mp4/h264 jest grywalny bez transkodowania; transkode i tak daje h264 mp4
        assert!(web_playable(&vid));
        let web = dir.join("web.mp4");
        assert!(transcode_web(&vid, &web, Some(dur), |_| {}));
        assert_eq!(probe(&web).unwrap().codec.as_deref(), Some("h264"));
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
