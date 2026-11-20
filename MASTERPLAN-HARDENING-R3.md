# MasterPlan — Hardening Round 3 (Quillon, final)

Tanggal: 2026-11-20 · Branch: `hardening/r3` · Baseline: main @ 0810796
+ PR #2 (`hardening/r2-core`, CI 4/4 hijau, 322 tests).

## Metode

Review 30 menit dengan timer Python (`scripts/review_timer.py`),
fokus: (a) verifikasi silang hasil R1+R2 — apakah guard lama bisa
dilewati atau mengalami regresi, (b) area yang belum tersentuh dua
round sebelumnya (core: subtitle/caption/time, audio mix, decoder,
preview watcher, CSV, canonical).

## Hasil verifikasi silang R1+R2 (tidak ada regresi/bypass)

- R2-1 `resolve()`: probe `a/./b`, `~/secrets`, `sub//x`, `..`,
  `\`, UNC, drive letter — semua tertangkap atau tetap di dalam
  root (bukti: /tmp/bypass_probe).
- R1 F1/F2/F9 + R2-3/R2-5: tes regresi lulus semua; fuzz 100k input
  lokal (`--ignored`, release) lulus 59 detik.
- CATATAN: PR #2 sempat merah di job `transcribe feature (linux)`
  (libclang absen di sandbox sehingga jalur itu tak pernah
  terkompilasi lokal) — diperbaiki dalam PR #2 (commits 97dbc1a,
  269b602), CI kini 4/4 hijau.

## Temuan R3 (terverifikasi dengan bukti)

| ID | Severity | Temuan | Bukti |
| --- | --- | --- | --- |
| R3-F1 | HIGH | `parse_timestamp` membatasi `m>59, sec>59, ms>999` tapi **jam tidak dibatasi**; `((h*60+m)*60+sec)*1000` overflow. SRT `9223372036854775807:59:59,999` → panic di subtitle.rs:85 (debug) / wrap negatif (release). Cakupan SRT+VTT (parser shared). | watch-it-fail: `quillon caption import` panic "attempt to multiply with overflow" |
| R3-F2 | MED | `ms_to_ticks` = `ms * timebase / 1000` unchecked mul; dipakai SRT/VTT import + whisper transcribe. Jam besar legal-parse menghasilkan ms yang meng-overflow saat konversi. | probe: crafted_ms.checked_mul(timebase) = None |
| R3-F3 | MED | `pipeline.rs:61` memanggil `frame_tick(doc.fps)` **sebelum** guard `frame <= 0` di baris berikutnya; `frame_tick(0)` = integer div-by-zero yang panic di **semua** profile. Jalur public API (`render_document_parallel` menerima `Document` apa pun). | probe catch_unwind: panicked "attempt to divide by zero" |
| R3-F4 | MED | `tick_to_sample` = `tick * rate / timebase` unchecked mul — **overflow untuk dokumen VALID** (24 h flicks × 192 kHz > i64::MAX → sampel negatif); dan `mix_document` mengalokasikan buffer penuh tanpa cap (24 h @ 48 kHz stereo = 33 GB; @ 192 kHz = 132,7 GB). | probe: dur.checked_mul(192_000) = None; i128 exact = 16,588,800,000 sampel = 132,7 GB |
| R3-F5 | INFO | Jalur library (`Renderer::new` tanpa validate) tidak memeriksa confinement asset src; semua jalur CLI/MCP memvalidasi lebih dulu. Catatan defense-in-depth, tidak di-eksekusi. | grep prepare.rs:84,117 |

Non-goal: tidak mengubah perilaku valid, tidak menambah dependency,
golden hashes tidak boleh bergeser.

## Steps eksekusi (checkpoint commit + verifikasi ketat)

- **S1 (R3-F1+F2)** — batasi jam timestamp ke nilai yang masuk akal
  (`MAX_TIMESTAMP_HOURS`, sejajar batas durasi dokumen 24 h + margin)
  dan buat `ms_to_ticks` checked (i128 mul + bound check, error
  bernama). Test watch-it-fail: SRT jam raksasa → error bernama,
  bukan panic. Jam 23:59:59,999 dan 100 jam masih legal-parse
  (parser), tapi konversi ke ticks menolak nilai melebihi batas.
- **S2 (R3-F3)** — cek `fps == 0` sebelum memanggil `frame_tick` di
  `render_document_parallel`; test: doc dengan fps 0 →
  `RenderError` bernama, bukan panic.
- **S3 (R3-F4)** — `tick_to_sample` checked (i128), dan
  `mix_document` menolak total sampel di atas `MAX_MIX_SAMPLES`
  (2^32 ≈ 4,3 miliar sampel ≈ 6,2 jam stereo 48 kHz ≈ 34 GB) dengan
  error bernama. Test: konversi exact untuk nilai besar legal,
  kegagalan bernama untuk melebihi cap.
- **S4** — laporan ringkasan tiga round di `docs/HARDENING.md`
  (tabel guard per round + statistik).
- **S5** — battery penuh + golden + fmt + clippy, push, PR.

## Definisi selesai (checklist)

- [ ] S1–S3 masing-masing dengan test yang gagal sebelum fix (yang
      bisa diekspresikan sebagai test)
- [ ] `cargo test --workspace` hijau; jumlah test ≥ 322 (bertambah)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` bersih
- [ ] `cargo fmt --all -- --check` bersih
- [ ] Golden hashes tidak berubah
- [ ] `docs/HARDENING.md` memuat baris guard R3 + ringkasan 3 round
- [ ] Branch `hardening/r3` ter-push, PR final dibuka
- [ ] Deliverable tersalin ke `download/`
