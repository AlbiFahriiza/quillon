# Quillon Stability Review

**Tanggal:** 2026-11-21

**Commit yang ditinjau:** `f5d01d0` (`main`, setelah hardening R1–R3)
**Status:** belum layak diberi label stable; temuan prioritas tinggi masih menyentuh alokasi memori, jalur file MCP, dan konversi waktu audio.

## Ringkasan

Quillon punya pemisahan modul yang cukup jelas dan kontrak yang sengaja ditulis: `quillon-core` mengatur model dokumen/schema/validasi, `quillon-render` membuat frame deterministik, `quillon-media` dan `quillon-audio` menangani media, `quillon-cache` menyimpan frame, lalu `quillon-cli` merangkai CLI, preview, dan MCP. CI di `main` menjalankan format, Clippy, dan workspace tests di Linux, macOS, serta Windows; run terakhir pada commit ini sukses pada 2026-11-20 ([CI run](https://github.com/AlbiFahriiza/quillon/actions/runs/44752836759)).

Namun, batas ukuran per dokumen belum membentuk batas working set yang aman. Beberapa input valid masih bisa meminta alokasi multi-gigabyte, video decode paralel dapat menggandakan kerja, dan field audio yang lolos validasi masih masuk ke perkalian `i64` tanpa pemeriksaan. Di jalur MCP, pemeriksaan `..` dan path absolut belum menahan symlink. Ada pula risiko hasil parsial dilaporkan sukses dan output lama hilang saat render gagal.

Temuan di bawah berasal dari pembacaan source, manifest, workflow, dokumentasi proyek, dan sumber teknis primer. Tidak ada tes, build, atau Clippy lokal yang dijalankan. `cargo audit` tidak tersedia di lingkungan review ini. Saya membaca status CI yang sudah ada di GitHub; itu bukan pengganti pengujian lokal atau audit dependency.

Pemeriksaan statis lokal atas perubahan laporan bersih (`check_diff.py`, `scan_disclosure.py`, dan `git diff --check`). `arch_gate.py` juga melaporkan no findings; temuan dependency `xtask -> quillon-cli` di bawah menunjukkan gate itu belum memeriksa dependency manifest.

Prioritas **P1** berarti input valid atau permintaan paralel dapat membuat proses gagal, hasil salah, atau melampaui batas keamanan yang dinyatakan. **P2** berarti jalur gagal, release gate, atau kontrak arsitektur yang perlu dibereskan sebelum 1.0 final.

## Arsitektur yang ditinjau

| Bagian | Tanggung jawab dan kontrak yang terlihat |
| --- | --- |
| `quillon-core` | Model dokumen, schema, validasi, lint, waktu, import dan API yang ditandai publishable. Tidak bergantung pada crate internal lain. |
| `quillon-render` | Renderer CPU deterministik dan pipeline frame berurutan. Bergantung ke core; tidak mengimpor media. |
| `quillon-media` | `FrameSource` video melalui proses ffmpeg sidecar. Decode dilakukan saat frame diminta dan cache byte targetnya 64 MiB. |
| `quillon-audio` | Decode, resample, mix, analisis, dan WAV sementara untuk mux. |
| `quillon-cache` | Cache frame konten-alamat, RAM terbatas, file frame di direktori pilihan pemanggil. |
| `quillon-cli` | Orkestrasi CLI, output, preview localhost, batch, dan server MCP stdio. |
| `xtask` | Ekspor schema, gallery, dan benchmark untuk repo. Aturan dependency menyatakan hanya core dan render yang boleh dipakai. |

Pemisahan domain/render/media/audio/cache adalah dasar yang baik untuk dirawat. Titik temu paling berisiko adalah batas antara validasi dokumen dan alokasi render, antara nilai waktu tervalidasi dan konversi sample, serta antara path MCP dan filesystem. Renderer per-worker juga menyiapkan ulang aset, sehingga biaya memori mengikuti jumlah thread, bukan hanya ukuran dokumen.

## Hal yang sudah kuat

- Batas lapisan dinyatakan di `disciplined.yaml`, dan aset dokumen menggunakan canonical path confinement di core.
- CI main menjalankan fmt, Clippy, dan tests di tiga OS; semua action pada workflow dipin ke commit SHA.
- Kontrak schema, exit code, hash frame, dan kebijakan perubahan API ditulis eksplisit.
- Beberapa batas input memiliki error bernama dan regression tests; installer memakai staging lalu rename.
- Pencarian kode produksi tidak menemukan blok `unsafe`; pemanggilan ffmpeg/ffprobe memakai argumen terpisah, bukan shell interpolation.

## Temuan prioritas tinggi

### P1 — Working set render tumbuh mengikuti jumlah worker dan ukuran frame

Dokumen valid dapat memakai canvas sampai 7.680 × 7.680. Satu frame RGBA pada ukuran itu berukuran 225 MiB. Pipeline membangun batch berukuran `max(2 * workers, 20)` sebelum mengirim hasil melalui channel, sedangkan default worker count adalah jumlah paralelisme host. CLI hanya menolak worker count 0 dan tidak membatasi nilai maksimum. Pada 16 worker, satu batch 32 frame berarti sekitar 7 GiB untuk frame mentah saja. Channel berkapasitas 32 frame dapat tetap penuh dari batch sebelumnya saat producer sudah mengumpulkan batch berikutnya, sehingga puncaknya dapat mendekati 64 frame atau 14 GiB, belum termasuk buffer internal renderer dan aset. Selain itu, setiap worker membuat `Renderer` sendiri, yang memanggil `prepare` dan membaca, decode, serta menyiapkan seluruh aset gambar lagi. Ukuran byte aset tidak dibatasi: bila hash pin dipakai, core membaca file dengan `std::fs::read`; `SessionKeys` cache meng-hash semua aset; `prepare` membaca semua image/font. Cache juga meng-clone frame dan menyalin seluruh buffer ke file staging. Jadi asset count cap dan LRU cap bukan working-set cap. [OWASP API4](https://owasp.org/API-Security/editions/2023/en/0xa4-unrestricted-resource-consumption) memasukkan memori, jumlah proses, ukuran input, dan timeout sebagai batas yang harus ditetapkan.

**Bukti:** `crates/quillon-core/src/lib.rs:53`; `crates/quillon-cli/src/render_doc.rs:71-75,116-119`; `crates/quillon-render/src/pipeline.rs:53-59,77-105,128-131,157-164`; `crates/quillon-render/src/prepare.rs:32-69,115-121`; `crates/quillon-core/src/validate.rs:219-248`; `crates/quillon-cache/src/lib.rs:54-66,237-275`.

**Arah perbaikan:** tetapkan anggaran byte/pixel kerja yang eksplisit; pilih ukuran batch dari anggaran byte, bukan hanya jumlah thread; batasi worker efektif; bagikan aset immutable antar-worker dan pisahkan state font/thread-local. Tambahkan batas total kerja piksel × frame agar durasi, fps, dan ukuran maksimum tidak lolos sebagai kombinasi yang tak praktis.

### P1 — Geometri elemen valid bisa mengalokasikan lebih dari 3,5 GiB

Validator menerima lebar/tinggi elemen positif sampai `MAX_ELEMENT_GEOMETRY = 30.720`. `rasterize_rect` membuat pixmap dengan ukuran elemen sebelum compositor memotongnya ke canvas. Sebuah persegi 30.720 × 30.720 meminta 3.774.873.600 byte (sekitar 3,52 GiB); jalur solid kemudian menyalin buffer penuh lewat `to_vec`. Jalur gradient memakai `Pixmap::new(...).expect(...)`, sehingga kegagalan alokasi juga dapat menjadi panic. Image/video fit meresize ke box elemen yang sama sebelum compositing. Jadi elemen yang sebagian besar di luar canvas tetap bisa meminta buffer besar.

**Bukti:** `crates/quillon-core/src/lib.rs:88-92`; `crates/quillon-core/src/validate.rs:426-435,524-547,550-600`; `crates/quillon-render/src/lib.rs:432-475`; `crates/quillon-render/src/visuals.rs:120-127`; `crates/quillon-render/src/prepare.rs:132-159`.

**Arah perbaikan:** batasi pixel/buffer elemen sebelum alokasi dan rasterisasi hanya bagian yang terlihat, atau tolak geometri yang membuat area kerja melebihi kuota. Batas dimensi sumber gambar juga harus eksplisit: crate `image` 0.25.10 menyebut batas lebar/tinggi default tidak disetel dan `max_alloc` default bersifat non-strict untuk decoder tertentu ([dokumentasi `Limits`](https://docs.rs/image/0.25.10/image/struct.Limits.html)).

### P1 — Video menerima dimensi tanpa batas dan cache miss paralel memulai decode duplikat

`SidecarDecoder::open` hanya memeriksa bahwa dimensi ffprobe positif. Ukuran buffer frame dihitung dengan perkalian `usize` tanpa `checked_mul`, lalu setiap frame dialokasikan sebelum hasil chunk dimasukkan dan dipangkas ke budget cache. `decode_at` melepas mutex setelah cache miss, lalu setiap worker dapat menjalankan `decode_chunk(index)` sendiri. Range tidak disejajarkan ke chunk dan tidak ada single-flight; banyak worker pada frame awal dapat menjalankan ffmpeg dan menahan buffer yang tumpang tindih bersamaan. `cache_capacity().max(2)` juga memaksa decode setidaknya dua frame meski satu frame sendiri melampaui budget 64 MiB.

**Bukti:** `crates/quillon-media/src/decoder.rs:56-80,115-169,187-212,217-237`.

**Arah perbaikan:** tetapkan lebar/tinggi dan pixel maksimum sebelum membuka decoder; gunakan aritmetika checked untuk ukuran frame; jadikan cache budget sebagai batas peak decode, bukan hanya batas setelah decode; sejajarkan chunk dan deduplikasi permintaan decode yang sedang berjalan.

### P1 — Resampling audio dapat melampaui batas decode berkali-kali lipat

Batas `MAX_DECODED_SAMPLES` diterapkan pada output decoder, tetapi `resample` tidak menerapkan batas pada output. Ia membuat salinan planar dari input, menghitung `out_capacity` dari rasio sample rate, lalu mengalokasikan buffer output per kanal tanpa cap. Rasio yang diizinkan dokumen, 8 kHz ke 192 kHz, adalah 24×. Jalur same-rate juga mengembalikan `input.clone()`, sehingga menyalin seluruh buffer. Sesudahnya, batas mix `2^32` sample interleaved masih dapat meminta 16 GiB sebagai `f32` sebelum overhead; itu bukan batas aman untuk mesin umum. Komentar `HARDENING.md` menyebut ukuran ini ~34 GB, padahal `2^32 × 4` byte adalah 16 GiB.

**Bukti:** `crates/quillon-audio/src/lib.rs:23-27,176-216,270-301,335-343`; `docs/HARDENING.md:45-50`.

**Arah perbaikan:** tetapkan budget kerja audio dalam byte untuk decode + resample + mix secara gabungan; hitung ukuran output dengan operasi checked sebelum alokasi; hindari clone untuk same-rate; perbaiki nilai dokumentasi agar merefleksikan byte yang benar.

### P1 — Field waktu audio yang valid masih dapat overflow

R3 menambahkan `tick_to_sample_checked` untuk total mix, tetapi `mix_track` dan `fade_envelope` masih memakai `tick_to_sample`, yaitu `tick * rate / timebase` di `i64`. Validasi hanya menolak nilai negatif atau fade length nol; `start_at`, source range, dan fade dapat tetap sebesar `i64::MAX`. Input seperti itu dapat panic pada build debug atau wrap pada build release, lalu menempatkan track/fade di sample yang salah. Ini adalah celah yang tersisa tepat di sebelah perbaikan overflow R3.

**Bukti:** `crates/quillon-core/src/validate.rs:632-645,677-718`; `crates/quillon-audio/src/lib.rs:250-267,346-385`.

**Arah perbaikan:** batasi setiap waktu track/range/fade terhadap timeline dokumen atau gunakan konversi checked yang sama pada seluruh jalur audio; hasil overflow harus menjadi error bernama sebelum loop mix.

### P1 — Batas root MCP bisa dilewati memakai symlink

`QuillonServer::resolve` menolak path absolut dan komponen `..`, tetapi hasilnya hanya `base_dir.join(relative)`. Tidak ada pemeriksaan canonical path atau symlink setelah join. `probe_media` membuka hasil tersebut; `render_document` menerima path output yang sama. Symlink di dalam root yang menunjuk ke luar root dapat membuat operasi file melewati batas yang didokumentasikan. Aset dokumen punya pemeriksaan canonical di core, tetapi `Document` tetap menyimpan path relatif asli dan `prepare::read_asset` membukanya lagi dengan `base_dir.join(src)`. Jika direktori kerja dapat diubah bersamaan, symlink bisa diganti di antara validasi dan pembacaan.

**Bukti:** `crates/quillon-cli/src/mcp.rs:300-305,413-420,615-656`; `crates/quillon-core/src/validate.rs:208-216,1009-1015`; `crates/quillon-render/src/prepare.rs:115-121`.

**Arah perbaikan:** selesaikan path terhadap root dan pastikan target/parent kanonis masih di bawah root; untuk output yang belum ada, validasi parent dan cegah symlink swap saat membuka file.

## Temuan lain yang perlu ditutup sebelum stabil

### P2 — Header WAV terpotong untuk mix yang diterima

`write_wav` mengalikan jumlah sample dengan 2 lalu cast ke `u32` tanpa cek. Mix sampai `MAX_MIX_SAMPLES = 2^32` diterima, sehingga payload WAV bisa lebih dari 4 GiB, sedangkan field ukuran RIFF dan chunk adalah 4 byte. Nilai ukuran yang ditulis akan terpotong; output WAV tidak menyatakan panjang data sesungguhnya. Format RIFF mendefinisikan kedua field ukuran sebagai 4-byte integer ([spesifikasi RIFF Microsoft](https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-)).

**Bukti:** `crates/quillon-audio/src/lib.rs:270-301,438-462`.

**Arah perbaikan:** tolak ukuran yang tidak muat pada RIFF sebelum menulis, atau gunakan format extended seperti RF64 / streaming mux yang mendukung ukuran lebih besar.

### P2 — Render menulis langsung ke output akhir dan mux dapat meninggalkan output hilang

MP4 sink memakai overwrite ke path tujuan sebelum render selesai. Error saat menulis frame atau encode dapat meninggalkan file parsial dan menghancurkan output lama. Saat mux audio, file video yang selesai dipindah ke temp; bila spawn ffmpeg gagal, fungsi keluar sebelum cleanup dan output final tidak ada. Jalur PNG menulis frame bertahap, jadi error di tengah juga meninggalkan direktori parsial. Jika error keluar sebelum `Mp4Sink::finish`, handle `Child` dijatuhkan tanpa `wait`/`kill`; dokumentasi Rust menyatakan proses tetap berjalan setelah handle dijatuhkan ([`std::process::Child`](https://doc.rust-lang.org/std/process/struct.Child.html)).

**Bukti:** `crates/quillon-media/src/lib.rs:53-109,134-157`; `crates/quillon-cli/src/render_doc.rs:129-164,246-280`.

**Arah perbaikan:** render ke target staging unik di direktori output, selesaikan video/audio/manifest, lalu rename atomik; gunakan cleanup guard dan pertahankan output sebelumnya sampai commit sukses.

### P2 — Proses media dapat menunggu tanpa deadline

`ffprobe` dijalankan dengan `Command::output()` tanpa timeout, decoder menunggu stdout ffmpeg sampai EOF, dan encoder/mux menunggu `child.wait()` tanpa deadline. FIFO atau media yang membuat child proses macet dapat menahan CLI/MCP selamanya; tidak ada jalur cancellation yang mengakhiri proses. OWASP API4 memasukkan execution timeout dan jumlah proses sebagai batas resource.

**Bukti:** `crates/quillon-media/src/probe.rs:76-97`; `crates/quillon-media/src/decoder.rs:150-180`; `crates/quillon-media/src/lib.rs:134-146`; `crates/quillon-cli/src/render_doc.rs:274-280`.

**Arah perbaikan:** tetapkan timeout/kill policy untuk semua child process, tunggu/reap child di semua jalur, dan sediakan pembatalan job render yang juga mengakhiri proses encoder/decoder.

### P2 — Buffer stderr encoder menyimpan seluruh log, bukan hanya tail

`Mp4Sink` memakai `mpsc::channel()` tak terbatas untuk semua baris stderr. Thread drain mengirim seluruh output, lalu `drain_tail` baru mengumpulkan semua pesan ke `Vec` dan menyisakan tiga baris terakhir. Encoder yang menghasilkan log besar dapat menumbuhkan RAM sepanjang proses walaupun pengguna hanya menerima tail.

**Bukti:** `crates/quillon-media/src/lib.rs:102-103,160-181,185-203`.

**Arah perbaikan:** gunakan ring buffer bertopik 3–10 baris atau channel bounded yang membuang log terlama.

### P2 — Panic producer bisa membuat render parsial terlihat sukses

Pipeline mengirim error eksplisit bila `render_one` mengembalikan `Err`, tetapi producer berjalan di thread detached. Jika thread panic, channel ditutup tanpa `Err`. `render_document` hanya membaca sampai EOF; tidak membandingkan jumlah frame yang ditulis dengan `doc.frame_count()`, lalu tetap menulis manifest dan mengembalikan `Ok`.

**Bukti:** `crates/quillon-render/src/pipeline.rs:85-125`; `crates/quillon-cli/src/render_doc.rs:142-184`.

**Arah perbaikan:** jadikan penyelesaian producer membawa hasil `JoinHandle`/status; anggap EOF dengan jumlah frame kurang sebagai kegagalan dan jangan finalisasi output.

### P2 — Batas dua render MCP bukan reservasi atomik; panic membuat run selamanya `Running`

Handler menghitung jumlah run aktif di satu lock, melepas lock, lalu baru menyisipkan `Running` di lock berikutnya. SDK `rmcp` menjalankan handler request secara konkuren, jadi dua panggilan bersamaan dapat sama-sama lolos saat slot terakhir tersedia. Selain itu, background thread hanya mengganti status setelah `render_document` kembali; panic sebelum itu meninggalkan run `Running` dan slot terpakai selamanya. Tes yang ada memanggil dua tool berturut-turut lalu menolak yang ketiga, bukan mengirim request serentak.

**Bukti:** `crates/quillon-cli/src/mcp.rs:397-469,985-1045`; dispatch konkuren ada pada `rmcp-3.11.0/src/service.rs:1647-1690`.

**Arah perbaikan:** lakukan cek + reservasi slot dalam critical section yang sama sebelum spawn; pasang guard terminal state yang menulis `Failed` ketika thread panic.

### P2 — Batas read teks dapat ditembus file yang tumbuh saat dibaca

`read_to_string_capped` memeriksa metadata, membuka file, lalu `read_to_string` sampai EOF; ukuran baru diperiksa setelah seluruh String terbentuk. Bila file tumbuh atau path diganti di antara pemeriksaan dan pembacaan, cap membatasi hasil yang diterima tetapi tidak membatasi alokasi puncaknya.

**Bukti:** `crates/quillon-cli/src/read_capped.rs:24-48`.

**Arah perbaikan:** baca maksimum `max_bytes + 1` dari file handle yang sama, lalu tolak bila byte tambahan terbaca.

### P2 — Limit baris/field CSV diterapkan sesudah semua record dimaterialisasi

`parse_rows` lebih dulu memanggil `split_records`, yang membangun `Vec<Vec<String>>` untuk seluruh input. Batas 1 MiB per field dan 1 juta baris baru dicek kemudian. File CSV 512 MiB yang ditolak tetap dapat membesar jauh di memori sebelum validasi batas berjalan.

**Bukti:** `crates/quillon-cli/src/csv.rs:17-68,71-105`; batas file 512 MiB di `crates/quillon-cli/src/read_capped.rs:17-20`.

**Arah perbaikan:** terapkan field/row limits saat parser menghasilkan record, sebelum memasukkan data ke struktur hasil.

### P2 — Preview tidak memakai read cap dokumen

CLI render memakai `read_to_string_capped`, sedangkan `preview::build_state` memakai `std::fs::read_to_string` langsung. Setiap reload/save preview membaca ulang file seluruhnya tanpa cap 64 MiB.

**Bukti:** `crates/quillon-cli/src/preview.rs:63-92`; jalur CLI terbatasi di `crates/quillon-cli/src/main.rs:554-569`.

**Arah perbaikan:** satukan pembacaan dokumen melalui fungsi capped yang sama.

### P2 — Import OTIO menelan error waktu pada audio track

`rational_to_ticks` mengembalikan error untuk rate tidak valid atau nilai yang overflow. Video import meneruskan error itu, tetapi audio import memanggil `.unwrap_or(0)` pada start dan duration; duration lalu dipaksa minimal satu tick. Timeline audio malformed dapat lolos sebagai track mulai di tick 0 sepanjang satu tick, dan CLI memvalidasi serta menulis dokumen hasil yang salah.

**Bukti:** `crates/quillon-core/src/otio.rs:39-55,206-227`; `crates/quillon-cli/src/main.rs:753-769`.

**Arah perbaikan:** bedakan field yang memang tidak ada (boleh default) dari field yang ada tetapi invalid (harus error), dan tambahkan regression case untuk audio start/duration overflow.

### P2 — Cache frame disk dibaca penuh sebelum header atau ukuran diperiksa

`read_frame` memanggil `std::fs::read`, lalu menyalin lagi payload `bytes[8..]` sebelum mengecek apakah panjang cocok dengan dimensi. File cache rusak/buatan yang sangat besar dapat menghabiskan RAM; pemeriksaan checked multiplication hanya terjadi setelah dua alokasi itu.

**Bukti:** `crates/quillon-cache/src/lib.rs:279-315`.

**Arah perbaikan:** cek ukuran file dan header terlebih dahulu, terapkan maksimum frame bytes, lalu baca payload dengan `read_exact` terbatas tanpa salinan sementara penuh.

### P2 — `xtask` melanggar arah dependency yang dinyatakan

`disciplined.yaml` hanya mengizinkan `xtask -> core, render`, tetapi `crates/xtask/Cargo.toml` memiliki dependency langsung `quillon-cli` dan gallery memanggil `quillon_cli::render_doc::save_png`. Ini menarik seluruh crate CLI ke tool pengembangan dan menyimpang dari design note yang mengarahkan xtask ke executable. `arch_gate.py` tidak melaporkan pelanggaran ini karena pemeriksaannya tidak menangkap dependency manifest tersebut.

**Bukti:** `disciplined.yaml:24-48`; `crates/xtask/Cargo.toml:11-18`; `crates/xtask/src/main.rs:132`; `docs/design-notes/v1.0-beta.md:24-31`.

**Arah perbaikan:** pindahkan helper PNG ke lapisan yang diizinkan atau jalankan binary; perluas gate arsitektur untuk memeriksa dependency workspace di manifest.

### P2 — Tag release tidak menunggu CI dan build memakai token write tingkat workflow

CI hanya berjalan untuk push ke `main` dan pull request; release berjalan pada tag `v*` dan hanya menunggu build/checksum. Tag dapat menunjuk commit yang tidak menjalani tes, fmt, atau Clippy. Di workflow release, `contents: write` diberikan global sehingga job build dependency mendapat izin lebih dari yang diperlukan; GitHub menyarankan default read-only dan menaikkan izin pada job yang memang membuat release ([least privilege](https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication), [secure use](https://docs.github.com/en/actions/reference/security/secure-use)).

**Bukti:** `.github/workflows/ci.yml:3-6,34-50`; `.github/workflows/release.yml:9-14,19-53,81-112`.

**Arah perbaikan:** jalankan seluruh gate pada commit tag atau wajibkan release bergantung pada gate yang memverifikasi `github.sha`; set `contents: read` di tingkat workflow dan `contents: write` hanya pada job release.

### P2 — Tidak ada pemeriksaan advisory dependency otomatis yang terlihat

Workflow repo tidak memiliki job `cargo audit`/`audit-check` atau konfigurasi Dependabot yang terlihat. Karena tool `cargo audit` juga tidak terpasang di mesin review ini, review ini tidak menyatakan apakah lockfile memiliki advisory aktif.

**Bukti:** hanya ada `.github/workflows/ci.yml`, `pages.yml`, dan `release.yml`; tidak ada workflow/config Dependabot di `.github/`.

**Arah perbaikan:** tambahkan pemindaian RustSec advisories pada PR dan jadwal berkala, dengan proses pengecualian advisory yang mencatat alasan. Tool yang direkomendasikan proyek RustSec adalah [`cargo-audit`](https://github.com/RustSec/rustsec/tree/main/cargo-audit).

### P2 — Klaim MSRV 1.85 belum diverifikasi oleh CI

Workspace menyatakan `rust-version = 1.85` dan README menyebut Rust 1.85+, tetapi CI memakai toolchain `stable` saja. Cargo mendefinisikan `rust-version` sebagai versi yang didukung dan menyebut dukungan terverifikasi mencakup pengujian otomatis pada versi tersebut ([Cargo Rust version](https://doc.rust-lang.org/cargo/reference/rust-version.html)). Run stable yang lolos tidak membuktikan source dan dependency lock masih dapat dibangun pada 1.85.

**Bukti:** `Cargo.toml` (`workspace.package.rust-version`); `.github/workflows/ci.yml:20-23,45-47`.

**Arah perbaikan:** tambahkan job minimal-toolchain `1.85` yang menjalankan check/test target yang didukung; dokumentasikan bila MSRV hanya dijanjikan untuk sebagian crate/feature.

### P2 — Download model transkripsi tidak punya batas total waktu atau ukuran

Fitur opsional transcribe memakai connect timeout 30 detik, tetapi body disalin sampai EOF sebelum hash diperiksa; tidak terlihat read/overall timeout atau batas jumlah byte. Dokumentasi ureq 3.5.2 menyatakan timeout lain default-nya `None`, dan connect timeout tidak membatasi pembacaan body setelah koneksi selesai ([konfigurasi ureq](https://docs.rs/ureq/3.5.2/ureq/config/struct.ConfigBuilder.html)). URL dan SHA pinned mengurangi risiko konten salah, tetapi endpoint yang menggantung atau memberi body berlebih tetap dapat menahan operasi dan memakai disk.

**Bukti:** `crates/quillon-audio/src/transcribe.rs:107-124`.

**Arah perbaikan:** tetapkan ukuran maksimum, deadline total, batas read timeout, dan hapus staging file pada setiap error.

## Ketidaksesuaian versi yang diterbitkan

Satu-satunya release/tag yang ditemukan adalah `v1.0.0-beta.1` dari 2026-11-19. Tag itu mendahului merge hardening R1–R3 tanggal 2026-11-20; pemeriksaan source pada tag tidak memuat batas durasi dan geometri yang ada di `main` sekarang. Jadi pengguna binary beta yang diterbitkan belum menerima hardening yang saat ini dilaporkan CI hijau.

**Bukti:** `git describe --tags --always` menghasilkan `v1.0.0-beta.1-28-gf5d01d0`; `gh release list` hanya menampilkan beta tersebut; `git show v1.0.0-beta.1:crates/quillon-core/src/lib.rs` dibandingkan dengan `main`.

Sebelum menyebut produk stable, terbitkan beta baru dari commit yang telah melewati gate release, perbarui release notes/status README, dan pertahankan schema/API freeze yang diminta di `docs/STABILITY.md` serta design note beta (termasuk jendela freeze schema 30 hari).

## Urutan stabilisasi yang disarankan

1. Tutup alokasi multi-gigabyte: tetapkan budget byte untuk render, gambar, video, dan audio; cek semua ukuran dengan aritmetika checked; kurangi batch dari jumlah worker.
2. Tutup correctness/security boundary: pakai sample conversion checked untuk seluruh field audio; konfin path MCP secara canonical termasuk symlink; validasi jumlah frame sebelum menyatakan sukses.
3. Jadikan output dan job lifecycle transaksional: file staging + rename, cleanup guard, status gagal pada panic, dan reservasi slot MCP atomik.
4. Terapkan limit lebih awal pada input: CSV streaming, cap di preview, pembacaan cache bounded, dan timeout/ukuran untuk download model.
5. Pulihkan kontrak arsitektur: hapus dependency `xtask -> quillon-cli` dan buat architecture gate memeriksa manifest.
6. Kunci kontrak release: CI pada Rust 1.85 dan tag release, token write hanya pada job publish, audit dependency berkala, dan perbandingan API SemVer untuk permukaan publishable `quillon-core`.
7. Potong beta baru dan verifikasi artifact dari tag tersebut. Setelah semua P1/P2 ditutup dan masa freeze schema/API tercapai, baru pertimbangkan `1.0.0` final.

## Riset web dan sumber primer

- [Cargo: Rust version dan verifikasi MSRV](https://doc.rust-lang.org/cargo/reference/rust-version.html) — field `rust-version` menyatakan dukungan toolchain; verifikasi mencakup automation pada versi yang didukung.
- [Cargo: SemVer compatibility](https://doc.rust-lang.org/cargo/reference/semver.html) — panduan untuk menjaga API library yang diterbitkan.
- [GitHub Actions: GITHUB_TOKEN](https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication) — izin dapat dibatasi per workflow atau job; prinsipnya berikan izin minimum.
- [GitHub Actions: secure use](https://docs.github.com/en/actions/reference/security/secure-use) — menyarankan default token read-only dan peningkatan izin hanya bila dibutuhkan.
- [GitHub Actions: workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax) — menjelaskan pemisahan filter branch/tag pada event `push`.
- [OWASP API4:2023](https://owasp.org/API-Security/editions/2023/en/0xa4-unrestricted-resource-consumption) — daftar batas resource yang relevan: waktu, memori, file descriptors, proses, ukuran input, dan jumlah operasi.
- [Rust `image` 0.25.10: `Limits`](https://docs.rs/image/0.25.10/image/struct.Limits.html) — batas lebar/tinggi tegas tidak aktif secara default; batas alokasi decoder bersifat non-strict untuk beberapa decoder.
- [Microsoft RIFF format](https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-) — struktur ukuran RIFF dan chunk memakai field 4 byte.
- [Rust `std::process::Child`](https://doc.rust-lang.org/std/process/struct.Child.html) — dropping handle tidak menghentikan atau menunggu proses.
- [ureq 3.5.2: konfigurasi timeout](https://docs.rs/ureq/3.5.2/ureq/config/struct.ConfigBuilder.html) — timeout global dan response-body harus diatur terpisah dari timeout koneksi; default-nya tidak aktif.
- [RustSec `cargo-audit`](https://github.com/RustSec/rustsec/tree/main/cargo-audit) — tool untuk memeriksa advisories terhadap `Cargo.lock`; belum dijalankan karena tidak tersedia di lingkungan review.
