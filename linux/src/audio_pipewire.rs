use crate::audio_stream::{CallbackReader, NativeReceiver};
use anyhow::{Context, Result};
use pipewire as pw;
use pw::spa::{
    self,
    pod::{Object, Pod, Value, serialize::PodSerializer},
};
use std::{io::Cursor, thread::JoinHandle};

pub struct Output {
    stop: pw::channel::Sender<()>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Output {
    pub fn start(source: &NativeReceiver) -> Result<Self> {
        let reader = source.callback_reader()?;
        let (stop, commands) = pw::channel::channel();
        let worker = std::thread::Builder::new()
            .name("evoparseq-pipewire".into())
            .spawn(move || playback(reader, commands))?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    fn shutdown(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let _ = self.stop.send(());
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

fn playback(reader: CallbackReader, stop: pw::channel::Receiver<()>) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let quit = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |()| quit.quit());
    let context = pw::context::Context::new(&mainloop)?;
    let core = context
        .connect(None)
        .context("Could not connect to PipeWire")?;
    let stream = pw::stream::Stream::new(
        &core,
        "EvoParseq",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Game",
        },
    )?;
    let quit = mainloop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(reader)
        .state_changed(move |_, _, _, state| {
            if matches!(
                state,
                pw::stream::StreamState::Error(_) | pw::stream::StreamState::Unconnected
            ) {
                quit.quit();
            }
        })
        .process(|stream, reader| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = usize::try_from(buffer.requested()).unwrap_or(usize::MAX);
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.data().map_or(0, |bytes| {
                let capacity = bytes.len() / 4;
                let frames = if requested == 0 {
                    capacity.min(480)
                } else {
                    capacity.min(requested)
                };
                let bytes = &mut bytes[..frames * 4];
                // i16 accepts every bit pattern. An unaligned mapping is filled
                // with silence to avoid constructing an invalid sample slice.
                let (head, samples, tail) = unsafe { bytes.align_to_mut::<i16>() };
                if head.is_empty() && tail.is_empty() {
                    reader.render(samples);
                } else {
                    bytes.fill(0);
                }
                frames * 4
            });
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = 4;
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
    info.set_channels(2);
    let (pod, _) = PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )?;
    let mut params = [Pod::from_bytes(pod.get_ref()).context("Invalid PipeWire audio format")?];
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
}
