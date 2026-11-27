# Phaze virtual-display refresh probe

Tracks [issue #41](https://github.com/vampywiz17/EvoParseq/issues/41).

The corrected mirror hello requests 60 Hz by default. The earlier implementation
mistook physical-size words for refresh and sent 10,000 in the actual refresh
fields, meaning 1 Hz. The opt-in probe now changes the real refresh fields.
Live verification of the applied Windows mode remains required.

The field layout and Hz × 10,000 representation were corroborated by read-only
inspection of the installed Phaze 0.14.3 client. See
[the provenance and field table](NATIVE_REFERENCES.md#phaze-display-descriptor-refresh-fields).
This is an observed compatibility contract, not a public vendor wire specification.
The [stock client's documentation](https://help.phaze.app/articles/1046218051-mirroring-displays)
says mirror mode attempts to match client refresh rates, but it does not document
the wire fields used by this independent client.

## What is recorded

Enable the existing event log. All three clients use the same Rust parser and
emit `phaze_display_refresh` events:

- `observation.kind = requested`: zero-based request slot, descriptor ID, size,
  the two encoded words, intended Hz and whether the probe override was used.
  This is logged after the local carrier accepts the startup hello, not after
  host mode confirmation.
- `observation.kind = reported`: host output ID/name, size, active/primary flags,
  separate raw physical-size values, and the two independent refresh words at
  descriptor offsets 44 and 48. Each is Hz multiplied by 10,000; divide each
  individually, never one by the other.
- `reported_hz` and `reported_native_hz` decode the independent RefreshRate and
  NativeRefreshRate fields. A zero word is unknown/null independently of the
  other field. Raw values remain available in `refresh_scaled` and
  `native_refresh_scaled`.
- `schema_version = 2` identifies this corrected interpretation.
  `field_interpretation_verified` is true for the statically corroborated layout.
  `os_mode_verified` remains false: a host reply is not proof of Windows' applied
  mode and can be a snapshot from before a layout change completes.
- Old schema-1 logs (without `schema_version`) incorrectly labelled physical-size
  words at offsets 28 and 32 as refresh. Ignore their `candidate_hz` ratio.
  Observed old pairs such as 600/340 and 60/10 are not Hz ratios.

The existing synthetic physical-size metadata is kept unchanged by the corrected
rate probe; it is not a measurement of a client's physical monitor. The
diagnostic records it separately so it cannot be confused with refresh again.

The requested descriptor ID is not assumed to equal a returned host output ID.
Windows screen numbering and reply order are not used to invent a mapping.
Use output names, sizes and the Windows display UI to identify each screen;
temporarily requesting distinct resolutions can help when names are ambiguous.
Do not infer a mode match solely from equal resolutions.

## Controlled desktop test

Close EvoParseq completely before each launch. Enable Phaze Mirror display,
request two virtual displays, keep their resolutions and the stream FPS unchanged
throughout the comparison, and enable event logging.

The optional `EVOPARSEQ_PHAZE_DISPLAY_HZ` variable accepts only 60 and 120.
One value applies to every requested display; a comma-separated list must have
exactly one entry per requested display. For two displays:

| Value | First requested display | Second requested display |
| --- | --- | --- |
| unset | corrected 60 Hz request | corrected 60 Hz request |
| `60,60` | intended 60 Hz | intended 60 Hz |
| `60,120` | intended 60 Hz | intended 120 Hz |
| `120,60` | intended 120 Hz | intended 60 Hz |

These are intended requests, not a claim that the driver supports or applies
those modes at every resolution. Both current and native rate fields are set to
the intended value: 60 Hz is encoded as 600,000, and 120 Hz as 1,200,000. The override is not saved in preferences.
With mirroring off it is ignored, including malformed values. With mirroring on,
invalid/non-UTF-8 values or the wrong number of entries fail the Phaze connection
before its display hello rather than silently testing another rate.
No automatic reconnect, runtime mode write or fallback layout is added.

Windows PowerShell, from the folder containing the client:

```powershell
$env:EVOPARSEQ_PHAZE_DISPLAY_HZ = '60,120'
.\EvoParseq.exe
```

Quit the client before the next test and change the variable. Remove it to
return to the unchanged default request:

```powershell
Remove-Item Env:\EVOPARSEQ_PHAZE_DISPLAY_HZ
```

Linux Flatpak uses the documented per-launch
[`--env` option](https://docs.flatpak.org/en/latest/flatpak-command-reference.html#flatpak-run):

```sh
flatpak run --env=EVOPARSEQ_PHAZE_DISPLAY_HZ=60,120 io.github.vampywiz17.EvoParseq
```

A later launch without `--env` restores the default unless a separate persistent
Flatpak override or inherited environment variable sets it. Remove that separate
override if present; this probe does not create one.

For each session, wait for the layout to settle, then record both the desktop
mode and active signal mode for **each** virtual display in Windows Advanced
display settings, plus the time and the client log. Record the initial values
before changing anything manually in Windows. Compare the baseline and the
two opposite mixed requests. A reported refresh of 120 with Windows still at 24 Hz
is a discrepancy to investigate, not proof of a successful mode change.

Video stream FPS remains a separate setting. Do not use received frames,
presentation intervals, configured client refresh, or host-reported fields as
a substitute for observing the host's applied display mode.

Windows/Linux live refresh validation remains pending. Android TV receives the
same passive diagnostics through the shared Rust client; the desktop shell
commands above do not constitute an Android TV user-facing control.
