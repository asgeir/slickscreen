use super::*;
use std::ops::Add;

use ffmpeg_next::codec::encoder;
use ffmpeg_next::codec::encoder::encoder::Encoder;
use ffmpeg_next::codec::encoder::video::Video;
use ffmpeg_next::codec::Context;
use ffmpeg_next::util::format::pixel::Pixel as ffmpeg_Pixel;
use ffmpeg_next::util::frame::Video as VideoFrame;

use crate::worker::WorkerControlMessage;

fn chunked_copy(dst: &mut [u8], dst_stride: usize, src: &[u8], src_stride: usize, width: usize) {
    if dst_stride == src_stride {
        dst.copy_from_slice(src);
    } else {
        assert!(dst_stride <= width);
        assert!(src_stride <= width);
        assert_eq!(dst.len() % dst_stride, 0);
        assert_eq!(src.len() % src_stride, 0);
        for (dst_row, src_row) in dst
            .chunks_exact_mut(dst_stride)
            .zip(src.chunks_exact(src_stride))
        {
            (dst_row[..width]).copy_from_slice(&src_row[..width]);
        }
    }
}

pub(super) enum VideoRecorderMessage {
    Quit,
}

impl From<worker::WorkerControlMessage> for VideoRecorderMessage {
    fn from(msg: WorkerControlMessage) -> Self {
        match msg {
            worker::WorkerControlMessage::Quit => VideoRecorderMessage::Quit,
        }
    }
}

pub(crate) struct VideoRecorder {
    pub worker: worker::Worker<VideoRecorderMessage>,
}

impl VideoRecorder {
    pub fn new(
        time_reference: SlickscreenTime,
        slickscreen_message_sender: SlickscreenMessageSender,
    ) -> Result<Self, SlickscreenError> {
        let display = scrap::Display::primary()
            .map_err(|e| SlickscreenError::ScreenCaptureError(e.to_string()))?;
        let (display_width, display_height) = (display.width(), display.height());

        let encoder_context = Context::new();
        let mut encoder = Video(Encoder(encoder_context));
        // https://github.com/mirror/x264/blob/master/encoder/encoder.c
        // search for: /* Detect default ffmpeg settings and terminate with an error. */
        encoder.set_time_base(ffmpeg_next::util::rational::Rational::new(1, 1000000));
        encoder.set_format(ffmpeg_Pixel::YUV420P);
        encoder.set_width(display_width as u32);
        encoder.set_height(display_height as u32);
        encoder.set_gop(4096);
        encoder.set_max_b_frames(0);
        encoder.set_colorspace(ffmpeg_next::util::color::Space::BT709);
        encoder.set_color_range(ffmpeg_next::util::color::Range::JPEG);
        encoder.set_me_range(16);
        encoder.set_qmin(10);
        encoder.set_qmax(51);
        let mut encoder_options = ffmpeg_next::Dictionary::new();
        encoder_options.set("preset", "medium");
        encoder_options.set("tune", "zerolatency");
        encoder_options.set("level", "4.2");
        encoder_options.set("profile", "high");
        encoder_options.set("refs", "1");
        encoder_options.set("crf", "15");
        encoder_options.set("qdiff", "4");
        encoder_options.set("qcompress", "0.6");
        encoder_options.set("color_primaries", "bt709");
        encoder_options.set("color_trc", "bt709");
        let mut encoder = encoder
            .open_as_with(
                encoder::find_by_name("libx264").ok_or(SlickscreenError::VideoEncoderNotFound(
                    "not found".to_string(),
                ))?,
                encoder_options,
            )
            .map_err(|e| SlickscreenError::VideoEncoderNotFound(e.to_string()))?;

        let worker = worker::Worker::new(
            slickscreen_message_sender,
            move |worker_sender: SlickscreenMessageSender,
                  control_receiver: crossbeam::channel::Receiver<VideoRecorderMessage>| {
                use std::io::ErrorKind::WouldBlock;

                let display =
                    scrap::Display::primary().expect("failed to get display in worker thread");
                let mut capturer =
                    scrap::Capturer::new(display).expect("failed to initialize screen capturer");

                let frame = VideoFrame::new(
                    ffmpeg_Pixel::BGRA,
                    display_width as u32,
                    display_height as u32,
                );
                let mut converter = frame
                    .converter(ffmpeg_Pixel::YUV420P)
                    .expect("failed to create bgra -> yuv420 converter");
                drop(frame);

                loop {
                    let start_of_frame = std::time::Instant::now();
                    let expected_next_frame =
                        start_of_frame.add(std::time::Duration::from_micros(16666));

                    match capturer.frame() {
                        Ok(screen_buffer) => {
                            let now = time_reference.pts_now();

                            let screen_buffer_stride = screen_buffer.len() / display_height;
                            let pixel_size = 4;
                            let row_length = pixel_size * display_width;

                            let mut bgra_frame = VideoFrame::new(
                                ffmpeg_Pixel::BGRA,
                                display_width as u32,
                                display_height as u32,
                            );
                            let bgra_frame_stride = bgra_frame.stride(0);
                            chunked_copy(
                                bgra_frame.data_mut(0),
                                bgra_frame_stride,
                                &screen_buffer,
                                screen_buffer_stride,
                                row_length,
                            );

                            let mut frame = VideoFrame::new(
                                ffmpeg_Pixel::YUV420P,
                                display_width as u32,
                                display_height as u32,
                            );
                            if let Err(e) = converter.run(&bgra_frame, &mut frame) {
                                println!("Error while converting frame to yuv: {:?}", e);
                                return;
                            }
                            frame.set_pts(Some(now));

                            if let Err(e) = encoder.send_frame(&frame) {
                                println!("Error while encoding video frame: {}", e);
                                return;
                            }

                            let mut packet = ffmpeg_next::Packet::empty();
                            while let Ok(_) = encoder.receive_packet(&mut packet) {
                                if let Err(e) =
                                    worker_sender.send(SlickscreenMessage::Video(packet.clone()))
                                {
                                    println!("Unable to send encoded video packet. Video encoder worker exiting. - {}", e);
                                    return;
                                }
                            }
                        }
                        Err(ref e) if e.kind() == WouldBlock => {}
                        Err(_) => {
                            println!("Unrecoverable error while capturing frame");
                            return;
                        }
                    }

                    loop {
                        match control_receiver.try_recv() {
                            Ok(VideoRecorderMessage::Quit) => {
                                return;
                            }
                            Err(crossbeam::channel::TryRecvError::Disconnected) => {
                                println!("Upstream message queue has been closed. Exiting video worker thread.");
                                return;
                            }
                            Err(crossbeam::channel::TryRecvError::Empty) => {
                                break;
                            }
                        }
                    }

                    let now = std::time::Instant::now();
                    let sleep_duration = expected_next_frame.duration_since(now);
                    if !sleep_duration.is_zero() {
                        //std::thread::sleep(sleep_duration);
                    }
                }
            },
        );

        Ok(Self { worker })
    }
}
