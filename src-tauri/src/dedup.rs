//! Wykrywanie duplikatów: dokładnych (BLAKE3), wizualnie podobnych (pHash)
//! i serii zdjęć (burst).

use std::collections::HashMap;

use image::DynamicImage;
use rusqlite::Connection;

/// pHash (DCT) + dHash (gradient) z już zdekodowanej bitmapy.
pub fn perceptual_hashes(img: &DynamicImage) -> (i64, i64) {
    use image_hasher::{HashAlg, HasherConfig};
    let phash = HasherConfig::new()
        .hash_size(8, 8)
        .preproc_dct()
        .hash_alg(HashAlg::Mean)
        .to_hasher()
        .hash_image(img);
    let dhash = HasherConfig::new()
        .hash_size(8, 8)
        .hash_alg(HashAlg::Gradient)
        .to_hasher()
        .hash_image(img);
    (bytes_to_i64(phash.as_bytes()), bytes_to_i64(dhash.as_bytes()))
}

fn bytes_to_i64(b: &[u8]) -> i64 {
    let mut arr = [0u8; 8];
    arr[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
    i64::from_le_bytes(arr)
}

fn hamming(a: i64, b: i64) -> u32 {
    (a ^ b).count_ones()
}

// ── union-find ──────────────────────────────────────────────────────────

struct Uf(Vec<usize>);

impl Uf {
    fn new(n: usize) -> Self {
        Uf((0..n).collect())
    }
    fn find(&mut self, x: usize) -> usize {
        if self.0[x] != x {
            let root = self.find(self.0[x]);
            self.0[x] = root;
        }
        self.0[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

// ── skan ────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct DupMember {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: i64,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub taken_at: Option<i64>,
    pub thumb: Option<String>,
    pub blurhash: Option<String>,
}

#[derive(serde::Serialize)]
pub struct DupGroup {
    pub kind: String, // "exact" | "similar" | "burst"
    pub members: Vec<DupMember>,
}

struct Row {
    member: DupMember,
    hash: Option<Vec<u8>>,
    phash: Option<i64>,
    exif_date: bool,
}

fn load_rows(conn: &Connection) -> Vec<Row> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, path, name, kind, size, width, height, taken_at,
                    CASE WHEN thumb = 1 THEN lower(hex(hash)) END, blurhash, hash, phash,
                    exif_date
             FROM files WHERE status = 0 AND hash IS NOT NULL AND length(hash) > 0",
        )
        .expect("stmt");
    let rows = stmt
        .query_map([], |r| {
            Ok(Row {
                member: DupMember {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    kind: r.get(3)?,
                    size: r.get(4)?,
                    width: r.get(5)?,
                    height: r.get(6)?,
                    taken_at: r.get(7)?,
                    thumb: r.get(8)?,
                    blurhash: r.get(9)?,
                },
                hash: r.get(10)?,
                phash: r.get(11)?,
                exif_date: r.get(12)?,
            })
        })
        .expect("query");
    rows.filter_map(Result::ok).collect()
}

/// Pełny skan duplikatów. `threshold` = maks. odległość Hamminga pHash (0–7).
pub fn scan(conn: &Connection, threshold: u32) -> Vec<DupGroup> {
    let rows = load_rows(conn);
    let mut groups = Vec::new();

    // 1. Dokładne: identyczny BLAKE3 (zdjęcia i wideo)
    let mut by_hash: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        if let Some(h) = &row.hash {
            by_hash.entry(h).or_default().push(i);
        }
    }
    for indices in by_hash.values() {
        if indices.len() > 1 {
            groups.push(DupGroup {
                kind: "exact".into(),
                members: indices.iter().map(|&i| rows[i].member.clone()).collect(),
            });
        }
    }

    // 2. Podobne: pHash na reprezentantach unikalnej treści (1 na blake3).
    // Multi-index: 8 pasm po 8 bitów — pary o odległości ≤ 7 mają zawsze
    // co najmniej jedno identyczne pasmo (zasada szufladkowa).
    let threshold = threshold.min(7);
    let reps: Vec<usize> = by_hash.values().map(|v| v[0]).collect();
    let candidates: Vec<(usize, i64)> = reps
        .iter()
        .filter_map(|&i| rows[i].phash.map(|p| (i, p)))
        .collect();
    let mut uf = Uf::new(candidates.len());
    for band in 0..8u32 {
        let mut buckets: HashMap<u8, Vec<usize>> = HashMap::new();
        for (ci, (_, phash)) in candidates.iter().enumerate() {
            buckets
                .entry(((*phash as u64) >> (band * 8)) as u8)
                .or_default()
                .push(ci);
        }
        for bucket in buckets.values() {
            // ponytail: kubełki zdegenerowane (jednolite zdjęcia) pomijamy,
            // żeby nie wpaść w O(n²); pełny LSH gdy zacznie przeszkadzać
            if bucket.len() > 2000 {
                continue;
            }
            for (a, &ci) in bucket.iter().enumerate() {
                for &cj in &bucket[a + 1..] {
                    if uf.find(ci) != uf.find(cj)
                        && hamming(candidates[ci].1, candidates[cj].1) <= threshold
                    {
                        uf.union(ci, cj);
                    }
                }
            }
        }
    }
    let mut similar: HashMap<usize, Vec<usize>> = HashMap::new();
    for ci in 0..candidates.len() {
        let root = uf.find(ci);
        similar.entry(root).or_default().push(candidates[ci].0);
    }
    for indices in similar.values() {
        if indices.len() > 1 {
            groups.push(DupGroup {
                kind: "similar".into(),
                members: indices.iter().map(|&i| rows[i].member.clone()).collect(),
            });
        }
    }

    // 3. Serie: zdjęcia w odstępie ≤ 2 s (min. 3 w serii).
    // Tylko prawdziwe daty EXIF — mtime po kopiowaniu bywa identyczny
    // dla tysięcy plików i produkowałby fałszywe serie.
    let mut dated: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            rows[i].member.kind == 0 && rows[i].exif_date && rows[i].member.taken_at.is_some()
        })
        .collect();
    dated.sort_by_key(|&i| rows[i].member.taken_at);
    let mut serie: Vec<usize> = Vec::new();
    let flush = |serie: &mut Vec<usize>, groups: &mut Vec<DupGroup>| {
        if serie.len() >= 3 {
            groups.push(DupGroup {
                kind: "burst".into(),
                members: serie.iter().map(|&i| rows[i].member.clone()).collect(),
            });
        }
        serie.clear();
    };
    for &i in &dated {
        match serie.last() {
            Some(&prev)
                if rows[i].member.taken_at.unwrap() - rows[prev].member.taken_at.unwrap()
                    <= 2 =>
            {
                serie.push(i);
            }
            _ => {
                flush(&mut serie, &mut groups);
                serie.push(i);
            }
        }
    }
    flush(&mut serie, &mut groups);

    // największe grupy na górze, w obrębie sekcji
    groups.sort_by_key(|g| (g.kind.clone(), std::cmp::Reverse(g.members.len())));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn insert(
        conn: &Connection,
        path: &str,
        hash: &[u8],
        phash: Option<i64>,
        taken_at: Option<i64>,
        size: i64,
    ) {
        conn.execute(
            "INSERT INTO files (path,parent,name,kind,size,mtime,hash,phash,taken_at,thumb,exif_date)
             VALUES (?1,'',?1,0,?5,1,?2,?3,?4,1,?6)",
            rusqlite::params![path, hash, phash, taken_at, size, taken_at.is_some()],
        )
        .unwrap();
    }

    #[test]
    fn finds_exact_similar_and_burst() {
        let dir = std::env::temp_dir().join("medianest-test-dedup");
        let _ = std::fs::remove_dir_all(&dir);
        let conn = db::open(&dir).unwrap();

        // dokładne duplikaty: ten sam blake3
        insert(&conn, "a.jpg", b"h1", Some(0b1111), Some(1000), 100);
        insert(&conn, "a_kopia.jpg", b"h1", Some(0b1111), Some(5000), 100);
        // podobne: phash różni się 2 bitami od a.jpg
        insert(&conn, "a_maly.jpg", b"h2", Some(0b0011), Some(9000), 50);
        // niepowiązany
        insert(&conn, "inny.jpg", b"h3", Some(!0b1111i64), Some(20000), 70);
        // seria: 3 zdjęcia co 1 s
        insert(&conn, "s1.jpg", b"s1", None, Some(50000), 10);
        insert(&conn, "s2.jpg", b"s2", None, Some(50001), 10);
        insert(&conn, "s3.jpg", b"s3", None, Some(50002), 10);

        let groups = scan(&conn, 6);
        let exact: Vec<_> = groups.iter().filter(|g| g.kind == "exact").collect();
        let similar: Vec<_> = groups.iter().filter(|g| g.kind == "similar").collect();
        let burst: Vec<_> = groups.iter().filter(|g| g.kind == "burst").collect();

        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].members.len(), 2);

        assert_eq!(similar.len(), 1, "a.jpg i a_maly.jpg powinny się sparować");
        assert_eq!(similar[0].members.len(), 2);

        assert_eq!(burst.len(), 1);
        assert_eq!(burst[0].members.len(), 3);

        // wyższy próg nie łączy odległych hashy
        let groups = scan(&conn, 0);
        assert!(groups.iter().filter(|g| g.kind == "similar").count() == 0);
    }

    #[test]
    fn banding_finds_all_pairs_within_threshold() {
        // multi-index (8 pasm × 8 bitów) musi znaleźć KAŻDĄ parę o odległości ≤ 7
        // — porównanie z brute force na pseudolosowych hashach
        let dir = std::env::temp_dir().join("medianest-test-banding");
        let _ = std::fs::remove_dir_all(&dir);
        let conn = db::open(&dir).unwrap();
        let mut lcg: u64 = 12345;
        let mut next = || {
            lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lcg
        };
        let mut hashes = Vec::new();
        for i in 0..120 {
            let base = next();
            hashes.push(base as i64);
            // co czwarty hash dostaje bliskiego sąsiada (2-4 przestawione bity)
            if i % 4 == 0 {
                let flipped = base ^ (1 << (next() % 64)) ^ (1 << (next() % 64));
                hashes.push(flipped as i64);
            }
        }
        for (i, p) in hashes.iter().enumerate() {
            insert(&conn, &format!("f{i}.jpg"), format!("u{i}").as_bytes(), Some(*p), None, 1);
        }

        let groups = scan(&conn, 7);
        // zbuduj mapę: phash → indeks grupy similar
        let mut group_of: HashMap<i64, usize> = HashMap::new();
        for (gi, g) in groups.iter().filter(|g| g.kind == "similar").enumerate() {
            for m in &g.members {
                let p: i64 = conn
                    .query_row("SELECT phash FROM files WHERE id=?1", [m.id], |r| r.get(0))
                    .unwrap();
                group_of.insert(p, gi);
            }
        }
        // brute force: każda para ≤ 7 musi być w tej samej grupie
        for i in 0..hashes.len() {
            for j in i + 1..hashes.len() {
                if hamming(hashes[i], hashes[j]) <= 7 {
                    assert_eq!(
                        group_of.get(&hashes[i]),
                        group_of.get(&hashes[j]),
                        "para ({i},{j}) o odległości {} rozdzielona",
                        hamming(hashes[i], hashes[j])
                    );
                    assert!(group_of.contains_key(&hashes[i]), "para ({i},{j}) nieznaleziona");
                }
            }
        }
    }

    #[test]
    fn perceptual_hash_resized_close_distinct_far() {
        let hashes: Vec<(i64, i64)> = (0..6)
            .map(|seed| {
                let big = crate::indexer_test_image(1280, 960, seed);
                let small = big.thumbnail(320, 240);
                let (p_big, _) = perceptual_hashes(&big);
                let (p_small, _) = perceptual_hashes(&small);
                (p_big, p_small)
            })
            .collect();

        // przeskalowana kopia zawsze blisko oryginału
        for (i, (big, small)) in hashes.iter().enumerate() {
            assert!(
                hamming(*big, *small) <= 6,
                "ziarno {i}: kopia za daleko ({})",
                hamming(*big, *small)
            );
        }
        // różne obrazy przeważnie daleko (pojedyncza para bywa blisko przypadkiem)
        let mut far = 0;
        let mut pairs = 0;
        for i in 0..hashes.len() {
            for j in i + 1..hashes.len() {
                pairs += 1;
                if hamming(hashes[i].0, hashes[j].0) > 6 {
                    far += 1;
                }
            }
        }
        assert!(far * 10 >= pairs * 8, "tylko {far}/{pairs} par odległych");
    }
}
