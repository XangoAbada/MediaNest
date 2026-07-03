//! Narzędzie deweloperskie: generator testowej biblioteki + seed ustawień.
//!
//! cargo run --example devtool -- gen <folder> <liczba-zdjęć>
//! cargo run --example devtool -- setlib <folder-biblioteki> <app-data-dir>

use image::{Rgb, RgbImage};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => gen(&args[2], args[3].parse().unwrap()),
        Some("setlib") => setlib(&args[2], &args[3]),
        Some("stats") => stats(&args[2]),
        Some("errors") => errors(&args[2]),
        Some("videos") => videos(&args[2]),
        Some("bench") => bench(args[2].parse().unwrap()),
        Some("movetest") => {
            // dowód, że przenoszenie D: -> C: wymaga fallbacku (rename zawodzi)
            let src = std::path::Path::new("D:\\Projects\\MediaNest\\_movetest_src.tmp");
            let dst = std::env::temp_dir().join("_movetest_dst.tmp");
            std::fs::write(src, b"cross-volume payload").unwrap();
            let rename = std::fs::rename(src, &dst);
            println!("rename D:->C: => {rename:?}");
            if rename.is_err() {
                std::fs::copy(src, &dst).unwrap();
                std::fs::remove_file(src).unwrap();
                println!("fallback copy+remove => OK, treść: {:?}", std::fs::read_to_string(&dst));
            }
            std::fs::remove_file(&dst).ok();
        }
        Some("paths") => {
            let conn = medianest_lib::db::open(std::path::Path::new(&args[2])).unwrap();
            let lib = medianest_lib::db::get_setting(&conn, "library_path");
            println!("library_path = {lib:?}");
            let mut stmt = conn
                .prepare("SELECT path, parent FROM files LIMIT 10")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap();
            for row in rows.flatten() {
                println!("path='{}' parent='{}'", row.0, row.1);
            }
        }
        Some("scantest") => {
            let base = std::env::temp_dir().join("medianest-scantest");
            let _ = std::fs::remove_dir_all(&base);
            let conn = medianest_lib::db::open(&base).unwrap();
            medianest_lib::indexer::scan(&conn, std::path::Path::new(&args[2])).unwrap();
            let mut stmt = conn
                .prepare("SELECT parent, count(*) FROM files GROUP BY parent ORDER BY 2 DESC LIMIT 10")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .unwrap();
            for row in rows.flatten() {
                println!("{:6}  '{}'", row.1, row.0);
            }
        }
        Some("folders") => {
            let conn = medianest_lib::db::open(std::path::Path::new(&args[2])).unwrap();
            let mut stmt = conn
                .prepare("SELECT parent, count(*) FROM files WHERE status=0 GROUP BY parent ORDER BY 2 DESC LIMIT 20")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .unwrap();
            for row in rows.flatten() {
                println!("{:6}  '{}'", row.1, row.0);
            }
        }
        Some("phashes") => {
            let conn = medianest_lib::db::open(std::path::Path::new(&args[2])).unwrap();
            let mut stmt = conn
                .prepare("SELECT path, phash FROM files WHERE phash IS NOT NULL ORDER BY path LIMIT 12")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .unwrap();
            for row in rows.flatten() {
                println!("{:016x}  {}", row.1 as u64, row.0);
            }
            let distinct: i64 = conn
                .query_row(
                    "SELECT count(DISTINCT phash) FROM files WHERE phash IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            println!("distinct phash: {distinct}");
        }
        Some("dedup") => {
            let conn = medianest_lib::db::open(std::path::Path::new(&args[2])).unwrap();
            let groups = medianest_lib::dedup::scan(&conn, 6);
            for kind in ["exact", "similar", "burst"] {
                let of_kind: Vec<_> = groups.iter().filter(|g| g.kind == kind).collect();
                println!(
                    "{kind}: {} grup, {} plików",
                    of_kind.len(),
                    of_kind.iter().map(|g| g.members.len()).sum::<usize>()
                );
            }
        }
        Some("rawprobe") => {
            let mut child = ffmpeg_sidecar::command::FfmpegCommand::new()
                .input(&args[2])
                .args(["-frames:v", "1", "-f", "null", "-"])
                .spawn()
                .unwrap();
            for event in child.iter().unwrap() {
                println!("{event:?}");
            }
        }
        Some("probe") => {
            let meta = medianest_lib::video::probe(std::path::Path::new(&args[2])).unwrap();
            println!("dur={:?} {:?}x{:?}", meta.duration, meta.width, meta.height);
        }
        _ => eprintln!("użycie: devtool gen <folder> <n> | devtool setlib <biblioteka> <app-data>"),
    }
}

/// Deterministyczne zdjęcie o WYRAŹNEJ strukturze niskoczęstotliwościowej
/// zależnej od ziarna (duży prostokąt + kierunek gradientu) — pHash pracuje
/// na niskich częstotliwościach, więc drobny szum nie różnicuje obrazów.
fn synth_photo(seed: u32, w: u32, h: u32) -> RgbImage {
    let s = seed.wrapping_mul(2654435761);
    let rx = (s >> 4) % (w / 2);
    let ry = (s >> 12) % (h / 2);
    let rw = w / 4 + ((s >> 20) % (w / 4));
    let rh = h / 4 + ((s >> 24) % (h / 4));
    let dir = s % 4;
    RgbImage::from_fn(w, h, |x, y| {
        let inside = x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let t = (match dir {
            0 => x * 255 / w,
            1 => y * 255 / h,
            2 => (x + y) * 255 / (w + h),
            _ => (x * 255 / w + 255 - y * 255 / h) / 2,
        }) as u8;
        if inside {
            Rgb([255 - t, (s >> 8) as u8, t])
        } else {
            Rgb([t, t / 2, (s >> 16) as u8])
        }
    })
}

fn gen(root: &str, n: u32) {
    let root = std::path::Path::new(root);
    for sub in ["2023/01", "2023/07", "2024/03", "2024/12", "rolka"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let subs = ["2023/01", "2023/07", "2024/03", "2024/12", "rolka"];
    for i in 0..n {
        let sub = subs[(i % subs.len() as u32) as usize];
        let img = synth_photo(i, 1280, 960);
        let path = root.join(sub).join(format!("IMG_{i:05}.jpg"));
        img.save(&path).unwrap();
        // co 10. zdjęcie ma dokładny duplikat, co 15. — przeskalowaną kopię
        if i % 10 == 0 {
            std::fs::copy(&path, root.join("rolka").join(format!("IMG_{i:05}_kopia.jpg")))
                .unwrap();
        }
        if i % 15 == 0 {
            synth_photo(i, 640, 480)
                .save(root.join("rolka").join(format!("IMG_{i:05}_small.jpg")))
                .unwrap();
        }
    }
    // atrapy wideo (nagłówek śmieciowy — indeksowane, miniaturki dopiero w Fazie 2)
    for i in 0..3u32 {
        let mut bytes = vec![0u8; 2 * 1024 * 1024];
        bytes
            .iter_mut()
            .enumerate()
            .for_each(|(j, b)| *b = (j as u32).wrapping_mul(i + 1) as u8);
        std::fs::write(root.join("rolka").join(format!("VID_{i:03}.mp4")), bytes).unwrap();
    }
    println!("wygenerowano bibliotekę testową w {}", root.display());
}

fn stats(app_data: &str) {
    let conn = medianest_lib::db::open(std::path::Path::new(app_data)).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT count(*), sum(hash IS NOT NULL), sum(thumb=1), sum(blurhash IS NOT NULL),
                    sum(taken_at IS NOT NULL), sum(error IS NOT NULL), sum(kind=1)
             FROM files WHERE status=0",
        )
        .unwrap();
    let row: (i64, i64, i64, i64, i64, i64, i64) = stmt
        .query_row([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })
        .unwrap();
    println!(
        "files={} hashed={} thumbs={} blurhash={} taken_at={} errors={} videos={}",
        row.0, row.1, row.2, row.3, row.4, row.5, row.6
    );
}

fn videos(app_data: &str) {
    let conn = medianest_lib::db::open(std::path::Path::new(app_data)).unwrap();
    let mut stmt = conn
        .prepare("SELECT path, duration, width, height, thumb FROM files WHERE kind=1")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<f64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .unwrap();
    for row in rows.flatten() {
        println!("{} dur={:?} {:?}x{:?} thumb={}", row.0, row.1, row.2, row.3, row.4);
    }
}

fn errors(app_data: &str) {
    let conn = medianest_lib::db::open(std::path::Path::new(app_data)).unwrap();
    let mut stmt = conn
        .prepare("SELECT path, error FROM files WHERE error IS NOT NULL LIMIT 20")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap();
    for row in rows.flatten() {
        println!("{} => {}", row.0, row.1);
    }
}

/// Benchmark pełnego pipeline'u: generuje N zdjęć, mierzy skan + indeksowanie
/// (hash, EXIF, miniaturka, blurhash, pHash) na wszystkich rdzeniach.
fn bench(n: u32) {
    let base = std::env::temp_dir().join("medianest-bench");
    let _ = std::fs::remove_dir_all(&base);
    let lib = base.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    println!("generuję {n} zdjęć testowych…");
    for i in 0..n {
        synth_photo(i, 1280, 960)
            .save(lib.join(format!("IMG_{i:06}.jpg")))
            .unwrap();
    }
    let conn = medianest_lib::db::open(&base.join("data")).unwrap();
    medianest_lib::db::set_setting(&conn, "library_path", lib.to_str().unwrap()).unwrap();

    let t0 = std::time::Instant::now();
    medianest_lib::indexer::scan(&conn, &lib).unwrap();
    let scan_time = t0.elapsed();

    let t1 = std::time::Instant::now();
    medianest_lib::indexer::process_all_pending(&conn, &lib, &base.join("thumbs"));
    let index_time = t1.elapsed();

    let done: i64 = conn
        .query_row("SELECT count(*) FROM files WHERE thumb=1", [], |r| r.get(0))
        .unwrap();
    println!(
        "skan: {:.2}s | indeksowanie {} plików: {:.2}s ({:.0} plików/s)",
        scan_time.as_secs_f64(),
        done,
        index_time.as_secs_f64(),
        done as f64 / index_time.as_secs_f64()
    );
}

fn setlib(library: &str, app_data: &str) {
    let conn = medianest_lib::db::open(std::path::Path::new(app_data)).unwrap();
    medianest_lib::db::set_setting(&conn, "library_path", library).unwrap();
    println!("library_path = {library}");
}
