`h264-40-frames.bin` contains the first 40 length-prefixed access units from
lowlat's synthetic `crates/decode/tests/data/synthetic-720p-h264.bin`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`:
https://github.com/nomi-san/lowlat

Copyright (c) 2026 Nguyen Duy. MIT license: see `LOWLAT-LICENSE`.
This offline parser regression fixture contains no captured user stream.

The HEVC fixtures are bounded prefixes from the same revision:
`hevc-40-frames.bin`: first 40 synthetic-720p-hevc units;
`hevc-nvenc-ll-8-frames.bin`: first 8 fixtures/hevc-nvenc-ll units;
`hevc-main10-first.bin` and `hevc-reordered-first.bin`: the first unit of
fixtures/hevc-main10 and fixtures/hevc-bframes, for explicit rejection tests.
They are synthetic/offline upstream fixtures, not captured account sessions.


`hevc-main10-lowlat-8-frames.bin` is a project-generated 64x64, 30fps testsrc2
pattern (8 access units, length-prefixed little-endian u32) for Main10 metadata
and reference-state tests, including repeated IDRs. It contains no captured
screen/account data. Generated with FFmpeg 7.1 / libx265 through the local-only
imageio-ffmpeg 0.6.0 tool bundle (not a project/runtime dependency):

```
ffmpeg -f lavfi -i testsrc2=size=64x64:rate=30 -frames:v 8 -pix_fmt yuv420p10le -c:v libx265 -preset ultrafast -tune zerolatency -x265-params bframes=0:rc-lookahead=0:ref=1:keyint=3:min-keyint=3:scenecut=0:repeat-headers=1:aud=1:pools=1:frame-threads=1 -f hevc main10-lowlat.hevc
```

Split Annex-B at access-unit delimiter NALs (type 35), retaining the delimiters
and all parameter sets; prefix each unit with its byte length. The original
`hevc-main10-first.bin` remains a rejection test: it declares frame reordering,
which is still deliberately unsupported in the live low-latency path.
