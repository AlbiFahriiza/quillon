//! Content-addressed frame cache (myquillonplan.md §4 F7, research/G §1).
//!
//! A frame is a pure function of (the document subtree that determines it,
//! the renderer version, the content hashes of the assets it uses, and the
//! scene-local tick). That tuple is the cache key, so editing one scene
//! changes only that scene's keys and the two invalidation axes — content
//! and renderer version — stay independent. Storage is a directory of
//! `<key>.frame` files (raw RGBA with a small header) plus a bounded
//! in-memory LRU; a cache failure is never a render failure, it just costs
//! a re-render.

use quillon_core::document::{Document, Element};
use quillon_core::frame_source::Frame;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Domain separation for the frame-key hash: bump when the key layout
/// itself changes (not when rendering changes — that is RENDERER_VERSION's
/// axis, passed in by the caller).
const KEY_DOMAIN: &str = "quillon-frame-key-v1";

/// A sha256 cache key.
pub type Key = [u8; 32];

/// Hit/miss counters; the acceptance log for incremental re-renders
/// (myquillonplan.md §12 v0.2: "verified from the cache hit/miss log").
#[derive(Debug, Default)]
pub struct CacheStats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub inserts: AtomicU64,
}

/// Per-render-session key material: asset content hashes computed once, and
/// which assets each scene uses. Asset identity is content, not path or
/// mtime (research/G): touching a file without changing it never
/// invalidates.
pub struct SessionKeys {
    renderer_version: u32,
    draft_scale: u32,
    asset_hashes: BTreeMap<String, Key>,
    scene_assets: Vec<BTreeSet<String>>,
}

impl SessionKeys {
    /// Hash every asset the document declares, once per session. `renderer`
    /// is the caller's RENDERER_VERSION constant (cache must not depend on
    /// the render crate); `draft_scale` is 0 for final frames and the
    /// divisor (e.g. 2) for draft frames, keeping the two apart in the key.
    pub fn new(
        doc: &Document,
        base_dir: &Path,
        renderer_version: u32,
        draft_scale: u32,
    ) -> io::Result<SessionKeys> {
        let mut asset_hashes = BTreeMap::new();
        for (id, asset) in &doc.assets {
            let bytes = std::fs::read(base_dir.join(asset.src()))?;
            let mut hasher = Sha256::new();
            hasher.update(b"quillon-asset");
            hasher.update(bytes);
            asset_hashes.insert(id.clone(), hasher.finalize().into());
        }
        let scene_assets = doc.scenes.iter().map(scene_asset_ids).collect();
        Ok(SessionKeys {
            renderer_version,
            draft_scale,
            asset_hashes,
            scene_assets,
        })
    }

    /// The cache key of the frame rendered from scene `scene_idx` at
    /// scene-local tick `local`. When the tick falls inside the scene's
    /// crossfade window the previous scene participates in the frame, so it
    /// is part of the key too.
    pub fn frame_key(&self, doc: &Document, scene_idx: usize, local: i64) -> Key {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DOMAIN.as_bytes());
        put_u32(&mut hasher, self.renderer_version);
        put_u32(&mut hasher, self.draft_scale);
        // Frame-global document fields: everything render_tick reads that is
        // not inside a scene subtree.
        put_u32(&mut hasher, doc.size.w);
        put_u32(&mut hasher, doc.size.h);
        put_str(&mut hasher, doc.bg.as_hex());
        put_str(
            &mut hasher,
            &serde_json::to_string(&doc.color_space).unwrap_or_default(),
        );
        put_scene(&mut hasher, &doc.scenes[scene_idx]);
        put_u64(&mut hasher, local as u64);
        if let (Some(duration), Some(prev)) = (
            doc.scenes[scene_idx].crossfade_in(),
            scene_idx.checked_sub(1),
        ) && local < duration
        {
            put_scene(&mut hasher, &doc.scenes[prev]);
            put_u64(&mut hasher, (doc.scenes[prev].duration + local) as u64);
        }
        for id in &self.scene_assets[scene_idx] {
            put_str(&mut hasher, id);
            if let Some(hash) = self.asset_hashes.get(id) {
                hasher.update(hash);
            }
        }
        hasher.finalize().into()
    }
}

/// Asset ids a scene's elements reference (text fonts, image assets).
fn scene_asset_ids(scene: &quillon_core::document::Scene) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for element in &scene.elements {
        match element {
            Element::Text(t) => {
                ids.insert(t.font.clone());
            }
            Element::Image(i) => {
                ids.insert(i.asset.clone());
            }
            Element::Video(v) => {
                ids.insert(v.asset.clone());
            }
            Element::Rect(_) => {}
        }
    }
    ids
}

fn put_u32(hasher: &mut Sha256, v: u32) {
    hasher.update(v.to_le_bytes());
}

fn put_u64(hasher: &mut Sha256, v: u64) {
    hasher.update(v.to_le_bytes());
}

fn put_str(hasher: &mut Sha256, s: &str) {
    put_u64(hasher, s.len() as u64);
    hasher.update(s.as_bytes());
}

fn put_scene(hasher: &mut Sha256, scene: &quillon_core::document::Scene) {
    // serde_json serializes struct fields in declaration order and maps as
    // BTreeMap, so this is byte-stable for equal scenes.
    if let Ok(json) = serde_json::to_string(scene) {
        put_str(hasher, &json);
    }
}

/// The frame cache: a directory of content-addressed frame files plus a
/// bounded in-memory LRU in front of it. Cheap to share across threads.
#[derive(Clone)]
pub struct FrameCache {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    mem_cap_bytes: usize,
    mem: Mutex<MemoryLru>,
    pub stats: CacheStats,
}

impl FrameCache {
    /// Open (and create) a cache directory. `mem_cap_bytes` bounds the
    /// in-memory layer; the disk layer is bounded only by the caller.
    pub fn open(dir: &Path, mem_cap_bytes: usize) -> io::Result<FrameCache> {
        std::fs::create_dir_all(dir)?;
        Ok(FrameCache {
            inner: Arc::new(Inner {
                dir: dir.to_path_buf(),
                mem_cap_bytes,
                mem: Mutex::new(MemoryLru {
                    entries: HashMap::new(),
                    bytes: 0,
                    tick: 0,
                }),
                stats: CacheStats::default(),
            }),
        })
    }

    /// Look a frame up: memory first, then disk (a disk hit promotes into
    /// memory). Counts one hit or one miss.
    pub fn get(&self, key: &Key) -> Option<Frame> {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        {
            let mut mem = self.inner.mem.lock().expect("cache lock poisoned");
            if let Some(frame) = mem.get(key) {
                drop(mem);
                self.inner.stats.hits.fetch_add(1, Ordering::Relaxed);
                return Some(frame);
            }
        }
        match Self::read_frame(&self.inner.dir.join(format!("{hex}.frame"))) {
            Ok(Some(frame)) => {
                let mut mem = self.inner.mem.lock().expect("cache lock poisoned");
                mem.insert(*key, frame.clone(), self.inner.mem_cap_bytes);
                self.inner.stats.hits.fetch_add(1, Ordering::Relaxed);
                Some(frame)
            }
            _ => {
                self.inner.stats.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Store a frame. Disk write failures are logged by returning the error
    /// to the caller but never block rendering: the memory layer still
    /// holds the frame for this session.
    pub fn put(&self, key: Key, frame: Frame) -> io::Result<()> {
        self.inner.stats.inserts.fetch_add(1, Ordering::Relaxed);
        let mut mem = self.inner.mem.lock().expect("cache lock poisoned");
        mem.insert(key, frame.clone(), self.inner.mem_cap_bytes);
        drop(mem);
        self.write_frame(&key, &frame)
    }

    /// (hits, misses) so far. This is the acceptance log.
    pub fn stats(&self) -> (u64, u64) {
        (
            self.inner.stats.hits.load(Ordering::Relaxed),
            self.inner.stats.misses.load(Ordering::Relaxed),
        )
    }

    fn file_for(&self, key: &Key) -> PathBuf {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        self.inner.dir.join(format!("{hex}.frame"))
    }

    /// Atomic on-disk store: write a temp file, then rename over the target
    /// so a concurrent reader never sees a partial frame.
    fn write_frame(&self, key: &Key, frame: &Frame) -> io::Result<()> {
        let target = self.file_for(key);
        let tmp = self.inner.dir.join(format!(
            "{}.tmp",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut body = Vec::with_capacity(8 + frame.rgba.len());
        body.extend_from_slice(&frame.width.to_le_bytes());
        body.extend_from_slice(&frame.height.to_le_bytes());
        body.extend_from_slice(&frame.rgba);
        std::fs::write(&tmp, body)?;
        // rename over an existing file is atomic on unix and Windows.
        std::fs::rename(&tmp, &target)?;
        Ok(())
    }

    fn read_frame(path: &Path) -> io::Result<Option<Frame>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        if bytes.len() < 8 {
            return Ok(None);
        }
        let width = u32::from_le_bytes(bytes[0..4].try_into().expect("4 bytes"));
        let height = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
        let rgba = bytes[8..].to_vec();
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return Ok(None);
        }
        Ok(Some(Frame {
            width,
            height,
            rgba,
        }))
    }
}

/// Approximate LRU: entries carry their byte cost; eviction removes the
/// least recently used entry (access order kept by re-insertion) until the
/// budget is met.
struct MemoryLru {
    entries: HashMap<Key, (Frame, u64)>,
    bytes: usize,
    tick: u64,
}

impl MemoryLru {
    fn frame_bytes(frame: &Frame) -> usize {
        frame.rgba.len() + 16
    }

    fn get(&mut self, key: &Key) -> Option<Frame> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(frame, last)| {
            *last = tick;
            frame.clone()
        })
    }

    fn insert(&mut self, key: Key, frame: Frame, cap: usize) {
        let cost = Self::frame_bytes(&frame);
        if cost > cap {
            return;
        }
        if let Some((old, _)) = self.entries.insert(
            key,
            (frame, {
                self.tick += 1;
                self.tick
            }),
        ) {
            self.bytes -= Self::frame_bytes(&old);
        }
        self.bytes += cost;
        while self.bytes > cap {
            match self
                .entries
                .iter()
                .min_by_key(|(_, (_, last))| *last)
                .map(|(k, _)| *k)
            {
                Some(oldest) => {
                    if let Some((frame, _)) = self.entries.remove(&oldest) {
                        self.bytes -= Self::frame_bytes(&frame);
                    }
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quillon_core::builder::DocumentBuilder;
    use quillon_core::color::Color;
    use quillon_core::document::{RectElement, Size};
    use quillon_core::time::FLICKS_PER_SECOND;

    fn three_scene_doc() -> Document {
        let scene = |id: &str, shade: u8| quillon_core::document::Scene {
            id: id.into(),
            duration: FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![Element::Rect(
                RectElement::new(0, 0, 64, 64).color(Color::from_rgb8(shade, shade, shade)),
            )],
        };
        DocumentBuilder::new(Size { w: 64, h: 64 })
            .scene(scene("s1", 40))
            .scene(scene("s2", 90))
            .scene(scene("s3", 200))
            .build()
    }

    fn keys_for(keys: &SessionKeys, doc: &Document) -> Vec<Vec<Key>> {
        (0..doc.scenes.len())
            .map(|i| {
                let frame = FLICKS_PER_SECOND / 30;
                (0..30).map(|f| keys.frame_key(doc, i, f * frame)).collect()
            })
            .collect()
    }

    #[test]
    fn same_scene_and_tick_produce_the_same_key() {
        let dir = std::env::temp_dir().join(format!("quillon-key-{}", std::process::id()));
        let doc = three_scene_doc();
        let keys = SessionKeys::new(&doc, &dir, 2, 0).unwrap();
        let a = keys.frame_key(&doc, 1, 12345);
        let b = keys.frame_key(&doc, 1, 12345);
        assert_eq!(a, b);
        // A different tick is a different frame.
        assert_ne!(a, keys.frame_key(&doc, 1, 12346));
    }

    #[test]
    fn editing_one_scene_changes_only_its_keys() {
        // Acceptance v0.2 #1 at key level: edit scene 2 (index 1), scenes 1
        // and 3 keep every key.
        let dir = std::env::temp_dir().join(format!("quillon-key-{}", std::process::id()));
        let doc = three_scene_doc();
        let before = {
            let keys = SessionKeys::new(&doc, &dir, 2, 0).unwrap();
            keys_for(&keys, &doc)
        };
        let mut edited = doc.clone();
        edited.scenes[1].elements.clear();
        let after = {
            let keys = SessionKeys::new(&edited, &dir, 2, 0).unwrap();
            keys_for(&keys, &edited)
        };
        assert_eq!(before[0], after[0], "scene 1 keys must not change");
        assert_eq!(before[2], after[2], "scene 3 keys must not change");
        assert_ne!(before[1], after[1], "scene 2 keys must change");
    }

    #[test]
    fn renderer_version_and_draft_are_independent_axes() {
        let dir = std::env::temp_dir().join(format!("quillon-key-{}", std::process::id()));
        let doc = three_scene_doc();
        let v2 = SessionKeys::new(&doc, &dir, 2, 0).unwrap();
        let v3 = SessionKeys::new(&doc, &dir, 3, 0).unwrap();
        let draft = SessionKeys::new(&doc, &dir, 2, 2).unwrap();
        let key = v2.frame_key(&doc, 0, 100);
        assert_ne!(key, v3.frame_key(&doc, 0, 100), "renderer version axis");
        assert_ne!(key, draft.frame_key(&doc, 0, 100), "draft axis");
    }

    #[test]
    fn crossfade_frames_key_on_both_scenes() {
        let dir = std::env::temp_dir().join(format!("quillon-key-{}", std::process::id()));
        let mut doc = three_scene_doc();
        doc.scenes[1].transition_in = Some(quillon_core::document::Transition {
            kind: quillon_core::document::TransitionKind::Crossfade,
            duration: FLICKS_PER_SECOND / 2,
        });
        let keys = SessionKeys::new(&doc, &dir, 2, 0).unwrap();
        let early = keys.frame_key(&doc, 1, 100);
        let late = keys.frame_key(&doc, 1, FLICKS_PER_SECOND);
        // Editing the previous scene must invalidate the crossfade frame but
        // not the post-crossfade frame.
        let mut edited = doc.clone();
        edited.scenes[0].elements.clear();
        let edited_keys = SessionKeys::new(&edited, &dir, 2, 0).unwrap();
        assert_ne!(
            early,
            edited_keys.frame_key(&edited, 1, 100),
            "crossfade frame depends on the previous scene"
        );
        assert_eq!(
            late,
            edited_keys.frame_key(&edited, 1, FLICKS_PER_SECOND),
            "a frame past the crossfade does not"
        );
    }

    #[test]
    fn disk_roundtrip_survives_a_new_instance() {
        let dir = std::env::temp_dir().join(format!(
            "quillon-cache-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let key = [7u8; 32];
        let frame = Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        };
        {
            let cache = FrameCache::open(&dir, 1024).unwrap();
            cache.put(key, frame.clone()).unwrap();
            assert_eq!(cache.get(&key), Some(frame.clone()));
        }
        // A fresh instance (empty memory layer) must hit from disk.
        let cache = FrameCache::open(&dir, 1024).unwrap();
        assert_eq!(cache.get(&key), Some(frame));
        let missing = cache.get(&[9u8; 32]);
        assert!(missing.is_none());
        let (hits, misses) = cache.stats();
        assert_eq!((hits, misses), (1, 1));
    }

    #[test]
    fn two_threads_share_the_cache_without_corruption() {
        // Plan §9.6: the same document rendered from two threads.
        let dir = std::env::temp_dir().join(format!(
            "quillon-cache-2t-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = FrameCache::open(&dir, 1 << 20).unwrap();
        let handles: Vec<_> = (0..2)
            .map(|t| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    for i in 0..64u32 {
                        // Key unique per (thread, i): the previous XOR scheme
                        // collided (0^53 == 1^52), which raced two frames
                        // onto one key — a test bug, not a cache bug.
                        let mut key = [t as u8; 32];
                        key[1] = i as u8;
                        let frame = Frame {
                            width: 1,
                            height: 1,
                            rgba: vec![t as u8, i as u8, 0, 255],
                        };
                        cache.put(key, frame.clone()).unwrap();
                        let got = cache.get(&key).expect("just-inserted frame");
                        assert_eq!(got.rgba, frame.rgba);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker thread");
        }
        // 128 inserts, 128 hits, 0 misses.
        assert_eq!(cache.stats(), (128, 0));
    }

    #[test]
    fn resume_after_partial_render_hits_only_completed_frames() {
        // Plan §9.6: an interrupted render resumes by cache key; frames
        // that finished are hits, the rest are misses.
        let dir = std::env::temp_dir().join(format!(
            "quillon-cache-resume-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = FrameCache::open(&dir, 1 << 20).unwrap();
        let frame = |i: u8| Frame {
            width: 1,
            height: 1,
            rgba: vec![i, i, i, 255],
        };
        for i in 0..10u8 {
            cache.put([i; 32], frame(i)).unwrap();
        }
        let mut hits = 0;
        let mut misses = 0;
        for i in 0..20u8 {
            match cache.get(&[i; 32]) {
                Some(_) => hits += 1,
                None => misses += 1,
            }
        }
        assert_eq!((hits, misses), (10, 10));
    }
}
