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
