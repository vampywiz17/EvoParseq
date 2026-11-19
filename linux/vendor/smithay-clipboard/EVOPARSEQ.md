Based on smithay-clipboard 0.7.3 (MIT), https://github.com/Smithay/smithay-clipboard.

Local extension: expose the existing clipboard data device and dispatch a private
native DnD MIME type through the existing worker. Clipboard transfers, selection
ownership and primary selection retain upstream behavior. This avoids competing
wl_data_device bindings after keyboard focus transitions on Mutter 46.
Callbacks must not access egui or block on clipboard requests. The Linux adapter
writes directly to the existing bounded input inbox and defers UI pointer reset
to the UI loop. No additional worker, event queue, timer or video path.
