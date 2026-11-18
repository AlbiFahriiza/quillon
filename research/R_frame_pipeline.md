# R. Frame-Feeding & Memory Notes — for Quillon

*Research date: 2026-11-17. Search findings (the dev.to deep-read timed out; pattern assembled from the ffmpeg-cookbook pipe article, Rust bounded-channel docs, and the search snippets).*

## The frame→encoder pipeline shape

Two viable paths for feeding rendered frames into encoding:

1. **Pipe mode (sidecar)**: spawn `ffmpeg` with `-f rawvideo -pix_fmt rgba -s WxH -r FPS -i -` on stdin; write raw frame bytes; close stdin to flush. Backpressure is free: the OS pipe buffer blocks the writer when ffmpeg's decoder/encoder is slower than the renderer. Pitfall from superuser#1708271: ffmpeg buffers until stdin closes for some muxers — use formats that stream (or write to a temp file for mp4 when needed) and test per-muxer.
2. **Libav mode (in-process)**: ffmpeg-next `frame → send_packet/receive_frame` ceremony (~40 lines, research/K) inside the media adapter; bounded mpsc channel between rayon render workers and the single encoder thread.

## Memory rules (the "never buffer the animation in RAM" rule)

- A 1080p RGBA frame is 8.3 MB; 30fps × 60s = 1,500 frames = **12.4 GB if buffered naively**. Therefore: **bounded channel** (capacity ≈ 2× workers) between renderers and encoder — the classic Rust bounded-channel backpressure pattern (rust-exercises.com; Microsoft Rust patterns book: bounded channels force the producer to wait, which is exactly the behavior wanted).
- Worker model: rayon renders frame N in parallel; results flow through the bounded channel in tick order (a reorder buffer sized ~2× workers for out-of-order completion); the encoder thread is the only writer to the output. Peak memory = workers × frame size + reorder buffer + encoder internal buffers — tens of MB, not GB.
- The preview server never holds more than the filmstrip window it is serving (content-addressed cache on disk, LRU in memory).
- Frame buffers reused via a small pool (Vec<u8> recycle) in v0.x only if profiling demands it — measure first (research/E §2 rule: measure before optimizing).

## Failure-first answers (systems-design applied to the render path)

- Encoder dies mid-render → workers see channel send error → render aborts with tick context in the error, partial manifest records completed frames (idempotent resume by cache key).
- Document references a missing asset → validation error before render starts (validate → probe → render, cheapest-first).
- Disk full during cache write → surface OS error with path; cache write failures are non-fatal (cache is an optimization, correctness comes from re-render).
- Render outruns encode (the normal case) → backpressure via bounded channel; workers park; no unbounded growth. This is the exact failure the dev.to article titles ("when rendering outruns encoding, write blocks instead of buffering your animation in RAM").
