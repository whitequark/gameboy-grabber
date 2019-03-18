#![allow(dead_code)]

#[macro_use]
extern crate serde_derive;
extern crate byteorder;
extern crate toml;
extern crate libusb;
extern crate flate2;
extern crate sdl2;
extern crate gif;
#[cfg(feature = "x264")]
extern crate x264;

use std::slice;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};
use std::io::{self, Read, BufReader, Write};
use std::fs::{self, File};
use std::thread::{self, JoinHandle};
use std::sync::mpsc::{channel, Sender, Receiver};
use byteorder::{NetworkEndian, ReadBytesExt, WriteBytesExt};

const VID_QIHW: u16        = 0x20b7;
const PID_GLASGOW: u16     = 0x9db1;

const REQ_TYPE_VENDOR: u8  = 0x40;

const REQ_EEPROM: u8       = 0x10;
const REQ_FPGA_CFG: u8     = 0x11;
const REQ_STATUS: u8       = 0x12;
const REQ_REGISTER: u8     = 0x13;
const REQ_IO_VOLT: u8      = 0x14;
const REQ_SENSE_VOLT: u8   = 0x15;
const REQ_ALERT_VOLT: u8   = 0x16;
const REQ_POLL_ALERT: u8   = 0x17;
const REQ_BITSTREAM_ID: u8 = 0x18;
const REQ_IOBUF_ENABLE: u8 = 0x19;
const REQ_LIMIT_VOLT: u8   = 0x1A;

const PORT_A: u16 = 0x01;
const PORT_B: u16 = 0x02;

const BUF_SIZE: usize = 16384;

struct Device(Receiver<Option<Vec<u8>>>);

impl Device {
    fn new(context: libusb::Context, bitstream: Option<Vec<u8>>, record: Option<File>)
          -> (Device, JoinHandle<()>) {
        let (sender, receiver) = channel();
        let thread = thread::spawn(move || {
            let mut handle = context.open_device_with_vid_pid(VID_QIHW, PID_GLASGOW)
                                    .expect("cannot open device");
            handle.write_control(REQ_TYPE_VENDOR, REQ_IO_VOLT, 0x00, PORT_A|PORT_B,
                                 &[0xe4, 0x0c], Default::default())
                  .expect("cannot set port AB voltage to 3V3");
            match bitstream {
                Some(bitstream) => {
                    for (index, chunk) in bitstream.chunks(1024).enumerate() {
                        handle.write_control(REQ_TYPE_VENDOR, REQ_FPGA_CFG, 0, index as u16, chunk,
                                             Default::default())
                              .expect("cannot download bitstream chunk");
                    }
                    handle.write_control(REQ_TYPE_VENDOR, REQ_BITSTREAM_ID, 0, 0, &[0xff; 16],
                                         Default::default())
                          .expect("cannot configure FPGA");
                }
                None => ()
            }
            handle.set_active_configuration(1)
                  .expect("cannot set configuration");
            handle.detach_kernel_driver(0)
                  .unwrap_or(/* ok if it didn't work */());
            handle.claim_interface(0)
                  .expect("cannot claim interface");

            let mut gzip = record.map(|file| {
                flate2::write::GzEncoder::new(file, flate2::Compression::fast())
            });

            let mut now = SystemTime::now();
            loop {
                let mut buf = Vec::new();
                buf.resize(BUF_SIZE, 0);
                match handle.read_bulk(0x86, &mut buf[..], Duration::from_millis(100)) {
                    Ok(size) => {
                        buf.resize(size, 0);
                        if let Some(gzip) = gzip.as_mut() {
                            let elapsed = now.elapsed().unwrap();
                            now = SystemTime::now();

                            gzip.write_u32::<NetworkEndian>(elapsed.subsec_nanos())
                                .expect("cannot write recording");
                            gzip.write_u32::<NetworkEndian>(buf.len() as u32)
                                .expect("cannot write recording");
                            gzip.write_all(&buf[..])
                                .expect("cannot write recording");
                        }
                        match sender.send(Some(buf)) {
                            Ok(()) => (),
                            Err(_) => break
                        }
                    }
                    Err(_) => {
                        match sender.send(None) {
                            Ok(()) => (),
                            Err(_) => break
                        }
                    }
                }
            }

            if let Some(gzip) = gzip {
                gzip.finish().expect("cannot finish recording");
            }
        });

        (Device(receiver), thread)
    }

    fn new_replay(file: File) -> (Device, JoinHandle<()>) {
        let (sender, receiver) = channel();
        let thread = thread::spawn(move || {
            let mut gzip = flate2::read::GzDecoder::new(file);

            loop {
                match gzip.read_u32::<NetworkEndian>() {
                    Ok(nanos) => thread::sleep(Duration::from_nanos(nanos as u64)),
                    Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    r => { r.expect("cannot read recording"); () }
                }
                let length = gzip.read_u32::<NetworkEndian>()
                                 .expect("cannot read recording") as usize;
                let mut data = Vec::new();
                data.resize(length, 0);
                gzip.read_exact(&mut data[..]).expect("cannot read recording");
                sender.send(Some(data))
                      .expect("cannot send buffer");
            }
            loop {
                thread::sleep(Duration::from_millis(100));
                sender.send(None)
                      .expect("cannot send buffer");
            }
        });

        (Device(receiver), thread)
    }
}

impl Read for Device {
    fn read(&mut self, dst_buf: &mut [u8]) -> io::Result<usize> {
        let src_buf = self.0.recv().expect("cannot receive buffer");
        match src_buf {
            Some(src_buf) => {
                assert!(dst_buf.len() >= src_buf.len());
                dst_buf[..src_buf.len()].copy_from_slice(&src_buf[..]);
                Ok(src_buf.len())
            }
            None => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "no data received"))
            }
        }
    }
}

struct VideoStream<R: Read> {
    reader:    R,
    sync_byte: Option<u8>,
    pitch:     usize,
}

struct Header {
    overflow: bool,
    n_frame:  usize,
    n_row:    usize
}

struct Scanline {
    header: Header,
    data:   Vec<u8> /* RGB */
}

impl<R: Read> VideoStream<R> {
    fn new(reader: R, pitch: usize) -> VideoStream<R> {
        VideoStream { reader, sync_byte: None, pitch }
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        if let Some(byte) = self.sync_byte.take() {
            return Ok(byte)
        }

        let mut byte = 0u8;
        self.reader.read(slice::from_mut(&mut byte))?;
        Ok(byte)
    }

    fn read_data_byte(&mut self) -> io::Result<u8> {
        let byte = self.read_byte()?;
        if byte & 0x80 == 0 {
            Ok(byte)
        } else {
            self.sync_byte = Some(byte);
            Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected sync byte"))
        }
    }

    fn read_header(&mut self) -> io::Result<Header> {
        let mut sync = 0u8;
        while sync & 0x80 == 0 {
            sync = self.read_byte()?;
        }
        let overflow = (sync & 0x40) >> 7;
        let n_frame  = (sync & 0x3e) >> 1;
        let n_row    = (sync & 0x01) << 7 | self.read_data_byte()?;
        Ok(Header {
            overflow: overflow != 0,
            n_frame:  n_frame as usize,
            n_row:    n_row as usize
        })
    }

    fn read_scanline(&mut self) -> io::Result<Scanline> {
        let header = self.read_header()?;
        let mut data = vec![0; self.pitch];
        for pixel in data.chunks_mut(3) {
            pixel[0] = self.read_data_byte()? << 3;
            pixel[1] = self.read_data_byte()? << 3;
            pixel[2] = self.read_data_byte()? << 3;
        }
        Ok(Scanline { header, data })
    }
}

#[derive(Debug, Default, Deserialize)]
struct Config {
    #[serde(rename = "device-type")]
    device_type: String,
    device: BTreeMap<String, DeviceConfig>,
    stream: Option<StreamConfig>,
    video: VideoConfig,
    // audio: AudioConfig,
}

#[derive(Debug, Default, Deserialize)]
struct DeviceConfig {
    bitstream: Option<String>,
    width: usize,
    height: usize,
}

#[derive(Debug, Default, Deserialize)]
struct StreamConfig {
    record: Option<String>,
    replay: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct VideoConfig {
    sdl: Option<SdlVideoConfig>,
    gif: Option<GifVideoConfig>,
    h264: Option<H264VideoConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct SdlVideoConfig {
    scale: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct GifVideoConfig {
    filename: String,
    #[serde(default)]
    framedrop: u8,
}

#[derive(Debug, Default, Deserialize)]
struct H264VideoConfig {
    filename: String
}

fn spawn_sdl_renderer(sdl_context: &sdl2::Sdl, width: usize, height: usize, factor: usize)
                     -> impl FnMut(&[u8]) {
    let video_subsystem = sdl_context
        .video()
        .expect("cannot initialize SDL video");
    let window = video_subsystem
        .window("Nintendo Game Boy", (width * factor) as u32, (height * factor) as u32)
        .build()
        .expect("cannot create SDL window");
    let mut canvas = window
        .into_canvas()
        .build()
        .expect("cannot create SDL canvas");
    let texture_creator = canvas
        .texture_creator();

    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    canvas.present();

    move |framebuffer| {
        // FIXME: kind of wasteful
        let mut texture = texture_creator
            .create_texture_streaming(PixelFormatEnum::RGB24, width as u32, height as u32)
            .expect("cannot create RGB555 SDL texture");

        texture.update(None, framebuffer, framebuffer.len() / height)
               .expect("cannot update texture");
        canvas.copy(&texture, None, None)
              .expect("cannot draw texture");
        canvas.present();
    }
}

fn spawn_gif_encoder(width: usize, height: usize, framedrop: u8,
                     filename: &str) -> Sender<Vec<u8>> {
    use gif::{Encoder, Parameter, Repeat, Frame};

    let image = File::create(filename).expect("cannot open GIF file");
    let mut encoder = Encoder::new(image, width as u16, height as u16, &[])
                              .expect("cannot create GIF encoder");
    Repeat::Infinite.set_param(&mut encoder)
           .expect("cannot set GIF repetition count");

    let (sender, receiver): (Sender<Vec<u8>>, _) = channel();
    thread::spawn(move || {
        loop {
            match receiver.recv() {
                Ok(framebuffer) => {
                    let mut frame = Frame::from_rgb(width as u16, height as u16, &framebuffer[..]);
                    frame.delay = 100 / (60 / (1 + framedrop) as u16);
                    encoder.write_frame(&frame).expect("cannot write frame");
                }
                Err(_) => break
            }
        }
    });
    sender
}

#[cfg(feature = "x264")]
fn spawn_x264_encoder(width: i32, height: i32, filename: &str)
                     -> (Sender<Vec<u8>>, JoinHandle<()>) {
    use x264::{Colorspace, Setup, Image, Preset, Tune};

    let mut file = File::create(filename).expect("cannot open H.264 file");

    let (sender, receiver): (Sender<Vec<u8>>, _) = channel();
    let thread = thread::spawn(move || {
        let mut encoder = Setup::preset(Preset::Veryslow, Tune::Animation, false, false)
            .fps(597, 10)
            .build(Colorspace::RGB, width, height)
            .expect("cannot create H.264 encoder");

        {
            let headers = encoder.headers().expect("cannot get H.264 headers");
            file.write_all(headers.entirety()).expect("cannot write H.264 headers");
        }

        let mut n = 0;
        loop {
            match receiver.recv() {
                Ok(framebuffer) => {
                    let image = Image::rgb(width, height, &framebuffer);
                    let (data, _) = encoder.encode(n, image).unwrap();
                    file.write_all(data.entirety()).expect("cannot write H.264 frame");
                    n += 1;
                }
                Err(_) => break
            }
        }

        {
            let mut flush = encoder.flush();
            while let Some(result) = flush.next() {
                let (data, _) = result.expect("cannot flush H.264 frame");
                file.write_all(data.entirety()).expect("cannot write H.264 delayed frame");
            }
        }
    });
    (sender, thread)
}

use sdl2::event::Event;
use sdl2::pixels::{Color, PixelFormatEnum};

fn main() {
    let config_file = fs::read_to_string("config.toml").expect("cannot open config.toml");
    let config: Config = toml::from_str(&config_file).expect("cannot parse config");

    let device = &config.device[&config.device_type];

    let (width, height) = (device.width, device.height);
    let pitch = width * 3;

    let context = libusb::Context::new().unwrap();
    let (device, device_thread) = match config.stream {
        Some(StreamConfig { replay: Some(filename), .. }) => {
            let replay_file = File::open(filename).expect("cannot open replay file");
            Device::new_replay(replay_file)
        }
        _ => {
            let record_file = match config.stream {
                Some(StreamConfig { record: Some(filename), .. }) =>
                    Some(File::create(filename).expect("cannot open record file")),
                _ => None
            };
            let bitstream = device.bitstream.as_ref().map(|path|
                fs::read(path).expect("cannot read bitstream"));
            Device::new(context, bitstream, record_file)
        }
    };
    let mut reader = VideoStream::new(BufReader::with_capacity(BUF_SIZE, device), pitch);

    let sdl_context = sdl2::init().expect("cannot initialize SDL");
    let mut event_pump = sdl_context
        .event_pump()
        .expect("cannot create SDL event pump");

    let mut sdl_video;
    match config.video.sdl {
        None => sdl_video = None,

        Some(SdlVideoConfig { scale }) => {
            let factor = scale.unwrap_or(1);
            sdl_video = Some(spawn_sdl_renderer(&sdl_context, width, height, factor))
        }
    }

    let mut gif_video;
    match config.video.gif {
        None =>
            gif_video = None,

        Some(GifVideoConfig { filename, framedrop }) =>
            gif_video = Some(spawn_gif_encoder(width, height, framedrop, &filename))
    }

    #[cfg(feature = "x264")]
    let mut h264_video;
    #[cfg(feature = "x264")]
    match config.video.h264 {
        None =>
            h264_video = None,

        Some(H264VideoConfig { filename }) =>
            h264_video = Some(spawn_x264_encoder(width as i32, height as i32, &filename))
    }

    let mut current_n_frame = 0;
    let mut current_n_row = 0;
    let mut framebuffer = vec![0u8; pitch * height];
    let mut framebuffer_full = framebuffer.clone();
    let mut skip_frame = false;
    'run: loop {
        match reader.read_scanline() {
            Ok(Scanline { header: Header { overflow, n_frame, n_row }, data }) => {
                if overflow {
                    print!("hardware reported FIFO overflow\n");
                }

                // LCDC outputs a 145th row and it's always white.
                // No idea what's up...
                if n_row == height { continue 'run }

                if n_row != (current_n_row + 1) % height {
                    print!("expected row {} got {}\n", (current_n_row + 1) % height, n_row);
                    skip_frame = true;
                }
                current_n_row = n_row;

                if n_frame != current_n_frame {
                    if n_frame != (current_n_frame + 1) % 32 {
                        print!("expected frame {} got {}\n", (current_n_frame + 1) % 32, n_frame);
                    }

                    // Keep requested FPS by duplicating the previous frame every time we skip one.
                    let full_frame;
                    if skip_frame {
                        skip_frame = false;
                        full_frame = framebuffer_full.clone();
                    } else {
                        framebuffer_full.copy_from_slice(&framebuffer[..]);
                        full_frame = framebuffer.clone();
                    }

                    if let Some(ref mut sdl_renderer) = sdl_video {
                        sdl_renderer(&full_frame);
                    }

                    if let Some(ref mut gif_encoder) = gif_video {
                        gif_encoder.send(full_frame.clone())
                                   .expect("cannot encode GIF frame");
                    }

                    #[cfg(feature = "x264")] {
                        if let Some((ref mut h264_encoder, _)) = h264_video {
                            h264_encoder.send(full_frame.clone())
                                       .expect("cannot encode H.264 frame");
                        }
                    }
                }
                current_n_frame = n_frame;

                framebuffer[n_row * pitch..(n_row + 1) * pitch].copy_from_slice(&data[..]);
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                let mut row = Vec::new();
                row.resize(pitch, 0);
                for (i, color) in [
                    [0xff, 0xff, 0xff],
                    [0xff, 0xff, 0x00],
                    [0x00, 0xff, 0xff],
                    [0x00, 0xff, 0x00],
                    [0xff, 0x00, 0xff],
                    [0xff, 0x00, 0x00],
                    [0x00, 0x00, 0xff],
                    [0x00, 0x00, 0x00],
                ].iter().enumerate() {
                    for j in 0..pitch / 24 {
                        row[i * pitch / 8 + j * 3..
                            i * pitch / 8 + (j + 1) * 3].copy_from_slice(color);
                    }
                }

                for n_row in 0..height {
                    framebuffer[n_row * pitch..(n_row + 1) * pitch].copy_from_slice(&row[..]);
                }

                if let Some(ref mut sdl_renderer) = sdl_video {
                    sdl_renderer(&framebuffer);
                }
            }
            Err(_) => {
                print!("stream synchronization lost\n");
                current_n_row = height - 1;
            }
        }

        for event in event_pump.poll_iter() {
            match event {
                Event::Quit {..} => break 'run,
                _ => ()
            }
        }
    }

    drop(reader);
    device_thread.join().expect("cannot join device thread");

    #[cfg(feature = "x264")] {
        if let Some((h264_encoder, h264_thread)) = h264_video {
            drop(h264_encoder);
            h264_thread.join().expect("cannot join H.264 thread");
        }
    }
}
