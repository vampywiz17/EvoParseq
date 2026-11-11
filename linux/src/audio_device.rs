//! One bounded PipeWire registry/parameter query before BUD negotiation.
//! No subprocess, device polling or persistent discovery thread.
use crate::audio_layout::Layout;
use pipewire as pw;
use pw::spa;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

pub fn positions(layout: Layout) -> [u32; 64] {
    use spa::sys::*;
    let channels: &[u32] = match (layout.channels, layout.mask) {
        (6, 0x60f) => &[
            SPA_AUDIO_CHANNEL_FL,
            SPA_AUDIO_CHANNEL_FR,
            SPA_AUDIO_CHANNEL_FC,
            SPA_AUDIO_CHANNEL_LFE,
            SPA_AUDIO_CHANNEL_SL,
            SPA_AUDIO_CHANNEL_SR,
        ],
        (6, _) => &[
            SPA_AUDIO_CHANNEL_FL,
            SPA_AUDIO_CHANNEL_FR,
            SPA_AUDIO_CHANNEL_FC,
            SPA_AUDIO_CHANNEL_LFE,
            SPA_AUDIO_CHANNEL_RL,
            SPA_AUDIO_CHANNEL_RR,
        ],
        (8, _) => &[
            SPA_AUDIO_CHANNEL_FL,
            SPA_AUDIO_CHANNEL_FR,
            SPA_AUDIO_CHANNEL_FC,
            SPA_AUDIO_CHANNEL_LFE,
            SPA_AUDIO_CHANNEL_RL,
            SPA_AUDIO_CHANNEL_RR,
            SPA_AUDIO_CHANNEL_SL,
            SPA_AUDIO_CHANNEL_SR,
        ],
        _ => &[SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR],
    };
    let mut result = [0; 64];
    result[..channels.len()].copy_from_slice(channels);
    result
}
fn from_positions(channels: &[u32]) -> Option<Layout> {
    for (count, mask) in [(2, 3), (6, 0x3f), (6, 0x60f), (8, 0x63f)] {
        let layout = Layout::new(count, mask).ok()?;
        let expected = positions(layout);
        if channels.len() == usize::from(count)
            && expected[..channels.len()]
                .iter()
                .all(|c| channels.contains(c))
        {
            return Some(layout);
        }
    }
    None
}
/// Unknown/default-unavailable never implies multichannel capability.
pub fn default_layout() -> anyhow::Result<Layout> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = Rc::new(context.connect(None)?);
    let registry = Rc::new(core.get_registry()?);
    let default = Rc::new(RefCell::new(None::<String>));
    let layouts = Rc::new(RefCell::new(HashMap::<String, Layout>::new()));
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let node_listeners = Rc::new(RefCell::new(Vec::new()));
    let metadata = Rc::new(RefCell::new(Vec::new()));
    let metadata_listeners = Rc::new(RefCell::new(Vec::new()));
    let reg = registry.clone();
    let selected = default.clone();
    let found = layouts.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            let Some(props) = &global.props else { return };
            if global.type_ == pw::types::ObjectType::Metadata
                && props.get("metadata.name") == Some("default")
            {
                if let Ok(proxy) = reg.bind::<pw::metadata::Metadata, _>(global) {
                    let selected = selected.clone();
                    let listener = proxy
                        .add_listener_local()
                        .property(move |subject, key, _, value| {
                            if subject == 0 && key == Some("default.audio.sink") {
                                *selected.borrow_mut() = value
                                    .filter(|v| v.len() <= 4096)
                                    .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                                    .and_then(|v| v["name"].as_str().map(str::to_owned));
                            }
                            0
                        })
                        .register();
                    metadata.borrow_mut().push(proxy);
                    metadata_listeners.borrow_mut().push(listener);
                }
            } else if global.type_ == pw::types::ObjectType::Node
                && props.get("media.class") == Some("Audio/Sink")
            {
                let Some(name) = props
                    .get("node.name")
                    .filter(|n| n.len() <= 1024)
                    .map(str::to_owned)
                else {
                    return;
                };
                if let Ok(node) = reg.bind::<pw::node::Node, _>(global) {
                    let found = found.clone();
                    let node = Rc::new(node);
                    let query = node.clone();
                    let listener = node
                        .add_listener_local()
                        .info(move |_| {
                            query.enum_params(0, Some(spa::param::ParamType::Format), 0, 1);
                        })
                        .param(move |_, id, _, _, pod| {
                            if id != spa::param::ParamType::Format {
                                return;
                            }
                            let Some(pod) = pod else { return };
                            let mut info = spa::param::audio::AudioInfoRaw::new();
                            if info.parse(pod).is_ok() && info.channels() <= 64 {
                                if let Some(layout) =
                                    from_positions(&info.position()[..info.channels() as usize])
                                {
                                    found.borrow_mut().insert(name.clone(), layout);
                                }
                            }
                        })
                        .register();
                    nodes.borrow_mut().push(node);
                    node_listeners.borrow_mut().push(listener);
                }
            }
        })
        .register();
    let rounds = Rc::new(Cell::new(0));
    let pending = Rc::new(Cell::new(core.sync(0)?));
    let seq = pending.clone();
    let connection = core.clone();
    let quit = mainloop.clone();
    let _done = core
        .add_listener_local()
        .done(move |id, current| {
            if id == pw::core::PW_ID_CORE && current == seq.get() {
                rounds.set(rounds.get() + 1);
                if rounds.get() >= 3 {
                    quit.quit();
                } else if let Ok(next) = connection.sync(0) {
                    seq.set(next);
                } else {
                    quit.quit();
                }
            }
        })
        .register();
    let quit = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| quit.quit());
    timer
        .update_timer(Some(Duration::from_secs(1)), None)
        .into_result()?;
    mainloop.run();
    let result = default
        .borrow()
        .as_ref()
        .and_then(|name| layouts.borrow().get(name).copied());
    Ok(result.unwrap_or(Layout::STEREO))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_audio_positions_use_explicit_windows_speaker_order_and_reject_unknown() {
        for (count, mask) in [(2, 3), (6, 0x3f), (6, 0x60f), (8, 0x63f)] {
            let layout = Layout::new(count, mask).unwrap();
            let mut pos = positions(layout)[..count as usize].to_vec();
            pos.reverse();
            assert_eq!(from_positions(&pos), Some(layout));
        }
        assert_eq!(from_positions(&[0; 6]), None);
        assert_eq!(from_positions(&[1; 8]), None);
    }
}
