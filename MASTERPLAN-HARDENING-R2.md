# MasterPlan — Hardening Round 2 (Quillon)

Tanggal: 2026-11-20 · Branch: `hardening/r2` · Baseline: main @ 0810796
(PR #1 ter-merge; 313 tests, clippy `-D warnings` clean, fmt clean.)

## Metode

Review 30 menit dengan timer Python (`scripts/review_timer.py`, bukan
scheduled task), fokus area yang tidak tersentuh R1: renderer pipeline,
preview server, cache, installer.sh, release workflow, xtask, fuzz
coverage. Setiap temuan di bawah diverifikasi terhadap source dengan bukti
(baris kode / program bukti / hitungan), bukan dugaan.

## Temuan (terverifikasi)

| ID | Severity | Temuan | Bukti |
| --- | --- | --- | --- |
| R2-1 | HIGH | `QuillonServer::resolve` hanya menolak `..`. Path absolut (`/etc/passwd`) menggantikan base sepenuhnya (semantik `Path::join`), lolos dari working root. Berlaku untuk `probe_media` path dan `render_document` out. | bukti kompilasi: `base.join("/etc/passwd")` → `/etc/passwd`, `starts_with(base)` = false |
| R2-2 | HIGH | Semua boundary baca file tidak dibatasi ukurannya: caption import (caption_cmd.rs:33), read_document (main.rs:555), OTIO (main.rs:730), template+CSV (batch.rs:40,48). File multi-GB masuk RAM penuh sebelum cap per-cue/CSV menyala. | grep semua `read_to_string` call site |
| R2-3 | MED | `quillon_audio::decode` men-decode seluruh file audio ke RAM tanpa batas sampel; 24 h @ 48 kHz stereo = ~33 GB f32 sebelum mixing jalan. | audio/src/lib.rs:147 `samples.extend_from_slice` di loop tanpa bound |
| R2-4 | MED | R1 F4 (cap 2 render konkuren + registry 32) tidak punya test regresi — jalur cap belum pernah dieksekusi CI. | grep: tidak ada test yang memanggil render ke-3 saat 2 berjalan |
| R2-5 | LOW | preview `encode_png` menelan error encode (`.ok()`) — frame gagal encode jadi PNG kosong tanpa jejak. | preview.rs:171-184 |
| R2-6 | LOW | whisper `.part` memakai nama sibling yang bisa ditebak (`model.part`) — kelas yang sama dengan R1 F5, di cache dir user. | transcribe.rs:98 |
| R2-7 | LOW | installer `cp` tidak atomik — dua instalasi bersamaan bisa meninggalkan binary setengah salin. | installer.sh:64 |
| R2-8 | LOW | Actions di workflows tidak dipin SHA — supply-chain hardening standar. | `.github/workflows/*.yml` |
| R2-9 | INFO | CI menjalankan fuzz 10k saja (100k `--ignored` lokal) — sesuai desain, dicatat saja. | ci.yml |
| R2-10 | INFO | `prepare()` membaca file font dua kali (probe family + font_system) — perf saja, di luar scope hardening. | prepare.rs:38,84 |

Non-goal: tidak mengubah behavior yang valid, tidak refactor, tidak
menyentuh golden hashes (kecuali memang bergeser karena bug — harus
dijelaskan), tidak menambah dependency baru.

## Steps eksekusi (checkpoint commit + verification ketat per step)

Setiap step: (1) tulis test yang gagal dulu (watch-it-fail), (2) perbaiki
di boundary, (3) test lulus, (4) `cargo test --workspace` + clippy
`-D warnings` + fmt, (5) golden hashes tidak bergeser, (6) commit
checkpoint dengan pesan `hardening(r2): ...`.

- **S1 (R2-1)** — `resolve()` menolak path absolut: scan
  `std::path::Component`, tolak `Prefix(_)` dan `RootDir` (menutup
  `/x` unix dan `C:\x` / `\\server` windows), pertahankan cek `..`.
  Test: probe_media dengan `/abs` dan `C:\abs` → error bernama.
- **S2 (R2-2)** — helper `read_capped(path, max_bytes)` di quillon-cli;
  dipakai di: caption import (64 MiB), document read (64 MiB), OTIO
  (64 MiB), template+CSV (batch; template 64 MiB, CSV 512 MiB).
  Error bernama menyebut limit dan ukuran aktual. Test per boundary.
- **S3 (R2-3)** — `MAX_DECODED_SAMPLES = 2^31` (~6.2 h stereo 48 kHz
  f32 ≈ 8.6 GB; jauh di atas materi nyata, tetap bounded). Cek di loop
  decode; error bernama. Test: mono 8 kHz besar → error, bukan OOM.
- **S4 (R2-4)** — test regresi MCP: dua render berjalan (dokumen berdurasi
  panjang), render ketiga → error "already running"; selesaikan, run baru
  sukses. Registry prune: 33+ run selesai → hanya 32 terbaru.
- **S5 (R2-5)** — `encode_png` mengembalikan `Result`; frame gagal
  dilewati dengan `eprintln!` operator-channel (bukan PNG kosong
  diam-diam).
- **S6 (R2-6)** — nama `.part` ditambah pid+nanos (sama pola R1 F5),
  create baru menolak path yang sudah ada.
- **S7 (R2-7)** — installer: salin ke `$DEST/quillon.tmp.$$` lalu `mv -f`
  (atomik di unix); Windows: rename via cmd — installer hanya unix+curl,
  `mv` cukup.
- **S8 (R2-8)** — pin semua `uses:` by full 40-char SHA (resolve via
  `gh api` ke commit tag terbaru yang valid).
- **S9** — update `docs/HARDENING.md` (tabel guard baru), jalankan
  battery penuh + golden, rapikan commit, push, buka PR.

## Definisi selesai (checklist)

- [ ] S1–S8 masing-masing punya test regresi yang gagal sebelum fix
- [ ] `cargo test --workspace` hijau; jumlah test ≥ 313 (bertambah)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` bersih
- [ ] `cargo fmt --all -- --check` bersih
- [ ] Golden hashes `testdata/golden/hashes.json` tidak berubah
- [ ] `docs/HARDENING.md` memuat baris guard baru
- [ ] Branch `hardening/r2` ter-push, PR dibuka ke main
- [ ] Deliverable tersalin ke `download/`
