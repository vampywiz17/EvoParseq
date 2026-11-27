# Offline HEVC regression fixture

`synthetic-720p-hevc.bin` is copied unchanged from lowlat revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, originally
`crates/decode/tests/data/synthetic-720p-hevc.bin`.
It contains 120 length-prefixed HEVC Main 8-bit 4:2:0 access units.
Copyright (c) 2026 Nguyen Duy; MIT license in `../LICENSE-lowlat`.

Only regression tests include this file, to verify bounded presentation leases
against an actual parsed HEVC reference-picture buffer. Normal client execution
does not include or play it. Standalone comparison executables have been retired.

`hevc-444-10.sps` is the first SPS NAL (Annex B start code included) from
`crates/decode/tests/data/fixtures/hevc-nvenc-444-10.bin` at the same revision
and under the same MIT license. It checks real 10-bit 4:4:4 metadata and surface
selection without bundling a complete clip. It is included by tests only;
metadata/profile checks do not establish successful hardware decoding.

`hevc-crop.sps` is only the first SPS NAL from lowlat commit
`2529df60b4b9f28c1e9a22a00db8abc15ed72589`,
`crates/decode/tests/data/fixtures/hevc-crop.bin` (MIT, same copyright above).
It describes 152x152 coded pixels and a 150x146 visible picture. Both independent
metadata readers are checked against it. Tests also synthesize a 1920x1088-coded /
1920x1080-visible header. No complete new clip or runtime diagnostic is added;
these metadata tests do not prove hardware decoding on an older AMF runtime.

See the [interoperability and test-data policy](../../../../docs/INTEROPERABILITY.md).
