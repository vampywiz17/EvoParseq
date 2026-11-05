use crate::audio_layout::Layout;
use crate::audio_stream::{CallbackReader, NativeReceiver};
use anyhow::{Context, Result};
use pipewire as pw;
use pw::spa::{
    self,
    pod::{Object, Pod, Value, serialize::PodSerializer},
};
use std::{
    cell::RefCell,
    io::Cursor,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
#[derive(Clone, Copy)]
pub struct Config {
    pub layout: Layout,
    pub minimum_ms: u32,
    pub maximum_ms: u32,
    pub custom: bool,
}
impl Config {
    pub fn new(settings: &crate::connection_settings::Settings, layout: Layout) -> Self {
        let (minimum_ms, maximum_ms) = if settings.custom_audio_buffer {
            (settings.audio_min_buffer_ms, settings.audio_max_buffer_ms)
        } else {
            (75, 150)
        };
        Self {
            layout,
            minimum_ms,
            maximum_ms,
            custom: settings.custom_audio_buffer,
        }
    }
}

#[derive(Default, Clone)]
pub struct State {
    pub commands: Option<pw::channel::Sender<Command>>,
    pub error: Option<String>,
    pub muted: Option<bool>,
    pub volume: Option<f32>,
    pub ready: bool,
    pub layout: Option<Layout>,
    pub limits: Option<(u32, u32, bool)>,
    pub recovering: bool,
    pub recoveries: u64,
}
#[derive(Clone, Copy)]
pub enum Command {
    Stop,
    Mute(bool),
    Volume(u8),
}
pub struct Output {
    stop: pw::channel::Sender<Command>,
    worker: Option<JoinHandle<Result<()>>>,
    cancelled: Arc<AtomicBool>,
}

impl Output {
    pub fn start(
        source: &NativeReceiver,
        state: std::sync::Arc<std::sync::Mutex<State>>,
        config: Config,
    ) -> Result<Self> {
        let reader = source.callback_reader()?;
        let (stop, commands) = pw::channel::channel();
        state.lock().unwrap_or_else(|e| e.into_inner()).commands = Some(stop.clone());
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let receiver = source.clone();
        let worker = std::thread::Builder::new()
            .name("evoparseq-pipewire".into())
            .spawn(move || {
                let reader = Rc::new(RefCell::new(reader));
                let mut commands = Some(commands);
                let mut recovery = crate::audio_recovery::Recovery::default();
                while !cancel.load(Ordering::Acquire) {
                    let result = playback(
                        reader.clone(),
                        &mut commands,
                        state.clone(),
                        config,
                        cancel.clone(),
                    );
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    receiver.discard_pending();
                    if !state.lock().unwrap_or_else(|e| e.into_inner()).recovering {
                        recovery.restored();
                    }
                    if recovery.begin() {
                        crate::event_log::record(
                            "WARN",
                            "audio_output_recovery_started",
                            serde_json::json!({"backend":"PipeWire"}),
                        );
                    }
                    {
                        let mut report = state.lock().unwrap_or_else(|e| e.into_inner());
                        report.ready = false;
                        report.recovering = true;
                        report.error = Some(
                            if result.is_err() {
                                "PipeWire output unavailable; retrying"
                            } else {
                                "PipeWire output disconnected; retrying"
                            }
                            .into(),
                        );
                    }
                    std::thread::park_timeout(recovery.failed());
                }
                Ok(())
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
            cancelled,
        })
    }

    fn shutdown(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        self.cancelled.store(true, Ordering::Release);
        let _ = self.stop.send(Command::Stop);
        worker.thread().unpark();
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("PipeWire worker stopped unexpectedly"))?
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn playback(
    reader: Rc<RefCell<CallbackReader>>,
    stop: &mut Option<pw::channel::Receiver<Command>>,
    state: std::sync::Arc<std::sync::Mutex<State>>,
    config: Config,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context
        .connect(None)
        .context("Could not connect to PipeWire")?;
    let quit_core = mainloop.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |_, _, _, _| quit_core.quit())
        .register();
    let stream = std::rc::Rc::new(pw::stream::Stream::new(
        &core,
        "EvoParseq",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Game",
        },
    )?);
    let quit_commands = mainloop.clone();
    let command_stream = stream.clone();
    let command_state = state.clone();
    let quit = mainloop.clone();
    let output_state = state.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .control_info(move |_, _, id, control| {
            let Some(control) = (unsafe { control.as_ref() }) else {
                return;
            };
            if control.n_values == 0 || control.n_values > 8 || control.values.is_null() {
                return;
            }
            let values =
                unsafe { std::slice::from_raw_parts(control.values, control.n_values as usize) };
            if values.iter().any(|v| !v.is_finite()) {
                return;
            }
            let mut state = output_state.lock().unwrap_or_else(|e| e.into_inner());
            if id == spa::sys::SPA_PROP_mute {
                state.muted = Some(values[0] != 0.);
            }
            if id == spa::sys::SPA_PROP_channelVolumes {
                state.volume = Some(values.iter().sum::<f32>() / values.len() as f32);
            }
        })
        .state_changed(move |_, _, _, stream_state| {
            let mut output = state.lock().unwrap_or_else(|e| e.into_inner());
            output.ready = matches!(stream_state, pw::stream::StreamState::Streaming);
            if output.ready {
                if output.recovering {
                    output.recoveries += 1;
                }
                output.recovering = false;
                output.error = None;
                output.layout = Some(config.layout);
                output.limits = Some((config.minimum_ms, config.maximum_ms, config.custom));
            }
            drop(output);
            let state = stream_state;
            if matches!(
                state,
                pw::stream::StreamState::Error(_) | pw::stream::StreamState::Unconnected
            ) {
                quit.quit();
            }
        })
        .process(move |stream, _| {
            let stride = usize::from(config.layout.channels) * 2;
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = usize::try_from(buffer.requested()).unwrap_or(usize::MAX);
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.data().map_or(0, |bytes| {
                let capacity = bytes.len() / stride;
                let frames = if requested == 0 {
                    capacity.min(480)
                } else {
                    capacity.min(requested)
                };
                let bytes = &mut bytes[..frames * stride];
                // i16 accepts every bit pattern. An unaligned mapping is filled
                // with silence to avoid constructing an invalid sample slice.
                let (head, samples, tail) = unsafe { bytes.align_to_mut::<i16>() };
                if head.is_empty() && tail.is_empty() {
                    reader.borrow_mut().render_layout(
                        samples,
                        config.layout,
                        config.minimum_ms as usize * 48,
                    );
                } else {
                    bytes.fill(0);
                }
                frames * stride
            });
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = (usize::from(config.layout.channels) * 2) as i32;
            *chunk.size_mut() = size as u32;
        })
        .register()?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(if cfg!(target_endian = "little") {
        spa::param::audio::AudioFormat::S16LE
    } else {
        spa::param::audio::AudioFormat::S16BE
    });
    info.set_rate(48_000);
    info.set_channels(u32::from(config.layout.channels));
    info.set_position(crate::audio_device::positions(config.layout));
    let (pod, _) = PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )?;
    let mut params = [Pod::from_bytes(pod.get_ref()).context("Invalid PipeWire audio format")?];
    let attached = stop
        .take()
        .context("Missing PipeWire command receiver")?
        .attach(mainloop.loop_(), move |command| {
            let result = match command {
                Command::Stop => {
                    cancel.store(true, Ordering::Release);
                    quit_commands.quit();
                    return;
                }
                Command::Mute(muted) => set_control(
                    &command_stream,
                    spa::sys::SPA_PROP_mute,
                    &mut [if muted { 1. } else { 0. }],
                ),
                Command::Volume(percent) => {
                    let gain = f32::from(percent.min(100)) / 100.;
                    set_control(
                        &command_stream,
                        spa::sys::SPA_PROP_channelVolumes,
                        &mut [gain; 8][..usize::from(config.layout.channels)],
                    )
                }
            };
            if result.is_err() {
                command_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .error = Some("PipeWire did not accept the audio control".into());
            }
        });
    let result = (|| -> Result<()> {
        stream.connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        mainloop.run();
        match stream.state() {
            pw::stream::StreamState::Error(_) => anyhow::bail!("PipeWire playback failed"),
            pw::stream::StreamState::Unconnected => anyhow::bail!("PipeWire playback disconnected"),
            _ => Ok(()),
        }
    })();
    *stop = Some(attached.deattach());
    result
}

// The pinned pipewire-rs 0.8 wrapper omits the C API's required trailing control
// ID 0. Call the public C API with an explicit sentinel on the PipeWire loop.
// The stack slice stays valid for the synchronous call; this never runs in process().
fn set_control(stream: &pw::stream::StreamRef, id: u32, values: &mut [f32]) -> Result<()> {
    anyhow::ensure!(
        !values.is_empty() && values.len() <= 8 && values.iter().all(|v| v.is_finite()),
        "Invalid PipeWire control"
    );
    let result = unsafe {
        pw::sys::pw_stream_set_control(
            stream.as_raw_ptr(),
            id,
            values.len() as u32,
            values.as_mut_ptr(),
            0u32,
        )
    };
    anyhow::ensure!(result >= 0, "PipeWire rejected the audio control");
    Ok(())
}
