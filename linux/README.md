# EvoParseq Linux preview

The Wayland desktop preview uses egui on Vulkan and shares account sign-in,
two-factor authentication and computer discovery with the Windows client.
The account worker holds credentials and session state in memory. Sign-out
revokes the session through the same shared account code.

Video capabilities lists driver-reported support for H.264 High, HEVC Main and
HEVC Main 10 (progressive 4:2:0). The probe checks codec queue support, profile
limits and formats that permit both decode output and shader sampling. It
enumerates all Vulkan adapters separately from the UI adapter selection.
Actual streaming, decode submission, presentation and PipeWire playback
still need to be implemented and tested.

The Flatpak uses the Freedesktop 25.08 runtime and its matching driver
extensions. The host supplies the Wayland compositor, kernel and compatible
GPU driver. No host filesystem access is required for the preview. CI stages
tracked sources and locked Cargo dependencies for an offline sandbox build.

Install the CI artifact on a Wayland desktop with Flathub configured:

```sh
flatpak install --user EvoParseq-Linux.flatpak
flatpak run io.github.vampywiz17.EvoParseq
flatpak run io.github.vampywiz17.EvoParseq --probe-video
```

For the streaming backend, select a physical device that can both decode and
present. Query compatible image usages and queue families before creating the
decoder and renderer. Sample decoded images in the presentation pass when
the device supports that combination. This is GPU-resident conversion, and
compositor or scanout copies require independent verification.
