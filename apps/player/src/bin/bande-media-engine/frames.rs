use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex};

use bande_media::{BufferKind, Channel, Event, FrameBuffer, Plane, VideoInfo};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use rustix::fs::{MemfdFlags, SealFlags, memfd_create};
use rustix::net::SendFlags;

const POOL_SIZE: usize = 4;
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const NV12: u32 = u32::from_le_bytes(*b"NV12");
const LINEAR_MODIFIER: u64 = 0;
const UDMABUF_DEVICE: &str = "/dev/udmabuf";
const UDMABUF_CREATE: libc::c_ulong = 0x4018_7542;
const UDMABUF_CLOEXEC: u32 = 1;
const PAGE_ROUND: u64 = 64 * 1024;

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

fn wrap_memfd(device: &File, memory: &File, size: u64) -> std::io::Result<OwnedFd> {
    let request = UdmabufCreate {
        memfd: memory.as_raw_fd() as u32,
        flags: UDMABUF_CLOEXEC,
        offset: 0,
        size,
    };
    let result = unsafe { libc::ioctl(device.as_raw_fd(), UDMABUF_CREATE, &request) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(result) })
}

struct Slot {
    id: u64,
    memory: File,
    busy: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Layout {
    width: u32,
    height: u32,
    fourcc: u32,
    strides: [u32; 2],
    lengths: [u64; 2],
}

impl Layout {
    fn planes(&self) -> Vec<Plane> {
        let mut planes = vec![Plane {
            offset: 0,
            stride: self.strides[0],
        }];
        if self.fourcc == NV12 {
            planes.push(Plane {
                offset: self.lengths[0] as u32,
                stride: self.strides[1],
            });
        }
        planes
    }

    fn size(&self) -> u64 {
        self.lengths[0] + self.lengths[1]
    }
}

#[derive(Default)]
struct Pool {
    layout: Option<Layout>,
    slots: Vec<Slot>,
    next_id: u64,
    shown: Option<Option<gst::ClockTime>>,
    frame_duration: Option<gst::ClockTime>,
    video: Option<VideoInfo>,
}

type VideoReporter = Box<dyn Fn(VideoInfo) + Send>;

pub struct Frames {
    device: Mutex<Option<File>>,
    channel: Arc<Channel>,
    pool: Mutex<Pool>,
    on_video: Mutex<Option<VideoReporter>>,
}

impl Frames {
    pub fn new(channel: Arc<Channel>) -> Self {
        Self {
            device: Mutex::new(None),
            channel,
            pool: Mutex::default(),
            on_video: Mutex::new(None),
        }
    }

    pub fn on_video(&self, report: VideoReporter) {
        if let Ok(mut on_video) = self.on_video.lock() {
            *on_video = Some(report);
        }
    }

    pub fn configure(&self, sink: &gst_app::AppSink, nv12_dmabuf: bool) {
        let device = nv12_dmabuf
            .then(|| {
                File::options()
                    .read(true)
                    .write(true)
                    .open(UDMABUF_DEVICE)
                    .inspect_err(|error| {
                        eprintln!("bande-media-engine: no {UDMABUF_DEVICE}: {error}")
                    })
                    .ok()
            })
            .flatten();
        if device.is_some() {
            let caps = gst_video::VideoCapsBuilder::new()
                .format(gst_video::VideoFormat::Nv12)
                .field("colorimetry", "bt709")
                .build();
            sink.set_caps(Some(&caps));
        }
        if let Ok(mut current) = self.device.lock() {
            *current = device;
        }
    }

    pub fn new_media(&self) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.shown = None;
            pool.video = None;
        }
    }

    pub fn released(&self, id: u64) {
        let Ok(mut pool) = self.pool.lock() else {
            return;
        };
        if let Some(slot) = pool.slots.iter_mut().find(|slot| slot.id == id) {
            slot.busy = false;
        }
    }

    pub fn frame_duration(&self) -> Option<gst::ClockTime> {
        self.pool.lock().ok()?.frame_duration
    }

    fn allocate(&self, layout: Layout) -> std::io::Result<(File, BufferKind, OwnedFd)> {
        let device = self.device.lock().map_err(|_| std::io::ErrorKind::Other)?;
        if layout.fourcc != NV12 {
            let memory = File::from(memfd_create("bande-media-frame", MemfdFlags::CLOEXEC)?);
            memory.set_len(layout.size())?;
            let shared = memory.as_fd().try_clone_to_owned()?;
            return Ok((memory, BufferKind::Shm, shared));
        }
        let Some(device) = device.as_ref() else {
            return Err(std::io::ErrorKind::Unsupported.into());
        };
        let memory = File::from(memfd_create(
            "bande-media-frame",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?);
        let size = layout.size().next_multiple_of(PAGE_ROUND);
        memory.set_len(size)?;
        rustix::fs::fcntl_add_seals(&memory, SealFlags::SHRINK)?;
        let shared = wrap_memfd(device, &memory, size)?;
        Ok((
            memory,
            BufferKind::Dmabuf {
                modifier: LINEAR_MODIFIER,
            },
            shared,
        ))
    }

    fn free_slot(&self, pool: &mut Pool, layout: Layout) -> std::io::Result<Option<usize>> {
        if pool.layout != Some(layout) {
            for slot in std::mem::take(&mut pool.slots) {
                self.channel
                    .send(&Event::Forget(slot.id), &[], SendFlags::empty())?;
            }
            pool.layout = Some(layout);
        }
        if let Some(free) = pool.slots.iter().position(|slot| !slot.busy) {
            return Ok(Some(free));
        }
        if pool.slots.len() >= POOL_SIZE {
            return Ok(None);
        }
        let (memory, kind, shared) = self.allocate(layout)?;
        let id = pool.next_id;
        pool.next_id += 1;
        let buffer = FrameBuffer {
            id,
            kind,
            width: layout.width,
            height: layout.height,
            fourcc: layout.fourcc,
            planes: layout.planes(),
        };
        self.channel.send(
            &Event::Buffer(buffer),
            &[shared.as_fd()],
            SendFlags::empty(),
        )?;
        pool.slots.push(Slot {
            id,
            memory,
            busy: false,
        });
        Ok(Some(pool.slots.len() - 1))
    }

    fn report_video(&self, pool: &mut Pool, info: &gst_video::VideoInfo) {
        let (numerator, denominator) = {
            let par = info.par();
            (par.numer().max(1) as u64, par.denom().max(1) as u64)
        };
        let video = VideoInfo {
            width: info.width(),
            height: info.height(),
            display_width: (u64::from(info.width()) * numerator / denominator) as u32,
        };
        if pool.video != Some(video) {
            pool.video = Some(video);
            if let Ok(on_video) = self.on_video.lock()
                && let Some(report) = on_video.as_ref()
            {
                report(video);
            }
        }
        let fps = info.fps();
        pool.frame_duration = (fps.numer() > 0).then(|| {
            gst::ClockTime::SECOND
                .mul_div_floor(fps.denom() as u64, fps.numer() as u64)
                .unwrap_or_default()
        });
    }

    fn present(&self, sample: &gst::Sample) -> Result<(), Box<dyn std::error::Error>> {
        let (Some(caps), Some(buffer)) = (sample.caps(), sample.buffer()) else {
            return Ok(());
        };
        let info = gst_video::VideoInfo::from_caps(caps)?;
        let mut pool = self.pool.lock().map_err(|_| "the frame pool is poisoned")?;
        let time = buffer.pts();
        if pool.shown == Some(time) {
            return Ok(());
        }
        self.report_video(&mut pool, &info);
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)?;
        let first = frame.plane_data(0)?;
        let second = match info.format() {
            gst_video::VideoFormat::Nv12 => frame.plane_data(1)?,
            _ => &[],
        };
        let strides = frame.plane_stride();
        let layout = Layout {
            width: info.width(),
            height: info.height(),
            fourcc: match second.is_empty() {
                true => XRGB8888,
                false => NV12,
            },
            strides: [
                u32::try_from(strides[0])?,
                strides
                    .get(1)
                    .map_or(Ok(0), |stride| u32::try_from(*stride))?,
            ],
            lengths: [first.len() as u64, second.len() as u64],
        };
        let Some(index) = self.free_slot(&mut pool, layout)? else {
            return Ok(());
        };
        let slot = &mut pool.slots[index];
        slot.memory.write_all_at(first, 0)?;
        slot.memory.write_all_at(second, first.len() as u64)?;
        slot.busy = true;
        let id = slot.id;
        pool.shown = Some(time);
        self.channel
            .send(&Event::Frame(id), &[], SendFlags::empty())?;
        Ok(())
    }

    fn sample(&self, sample: Result<gst::Sample, gst::glib::BoolError>) -> gst::FlowSuccess {
        let shown = sample
            .map_err(Box::<dyn std::error::Error>::from)
            .and_then(|sample| self.present(&sample));
        if let Err(error) = shown {
            eprintln!("bande-media-engine: cannot pass a frame: {error}");
        }
        gst::FlowSuccess::Ok
    }
}

pub fn video_sink(frames: Arc<Frames>, delay: gst::ClockTime) -> gst_app::AppSink {
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Bgrx)
        .build();
    let on_sample = Arc::clone(&frames);
    let sink = gst_app::AppSink::builder()
        .caps(&caps)
        .sync(true)
        .qos(true)
        .enable_last_sample(false)
        .max_buffers(2)
        .callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| Ok(frames.sample(sink.pull_preroll())))
                .new_sample(move |sink| Ok(on_sample.sample(sink.pull_sample())))
                .build(),
        )
        .build();
    sink.set_render_delay(delay);
    sink
}

pub fn subtitle_sink(channel: Arc<Channel>) -> gst_app::AppSink {
    let caps = gst::Caps::builder("text/x-raw")
        .field("format", gst::List::new(["utf8", "pango-markup"]))
        .build();
    gst_app::AppSink::builder()
        .caps(&caps)
        .sync(true)
        .enable_last_sample(false)
        .callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let Ok(sample) = sink.pull_sample() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let Some(buffer) = sample.buffer() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let text = buffer
                        .map_readable()
                        .ok()
                        .map(|map| String::from_utf8_lossy(map.as_slice()).into_owned());
                    let event = Event::Subtitle {
                        text: text.filter(|text| !text.is_empty()),
                        duration_ms: buffer.duration().map(gst::ClockTime::mseconds),
                    };
                    if channel.send(&event, &[], SendFlags::empty()).is_err() {
                        return Err(gst::FlowError::Eos);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        )
        .build()
}
