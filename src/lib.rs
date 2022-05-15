mod audio_recorder;
mod error;
mod util;
mod video_recorder;
mod worker;

pub use error::*;
use util::*;

use audio_recorder::*;
use video_recorder::*;

use cpal::traits::StreamTrait;
use cpal::traits::{DeviceTrait, HostTrait};

enum SlickscreenMessage {
    Quit,
    Audio(ffmpeg_next::codec::packet::Packet),
    Video(ffmpeg_next::codec::packet::Packet),
}

impl From<worker::WorkerControlMessage> for SlickscreenMessage {
    fn from(msg: worker::WorkerControlMessage) -> Self {
        match msg {
            worker::WorkerControlMessage::Quit => SlickscreenMessage::Quit,
        }
    }
}

type SlickscreenMessageSender = crossbeam::channel::Sender<SlickscreenMessage>;
type SlickscreenMessageReceiver = crossbeam::channel::Receiver<SlickscreenMessage>;

#[derive(Clone, Debug)]
pub struct SlickscreenConfig {
    pub output_file: Option<String>,
}

impl Default for SlickscreenConfig {
    fn default() -> Self {
        SlickscreenConfig { output_file: None }
    }
}

pub struct Slickscreen {
    worker: worker::Worker<SlickscreenMessage>,
    audio_recorder: AudioRecorder,
    video_recorder: VideoRecorder,
}

impl Slickscreen {
    pub fn new(config: SlickscreenConfig) -> Result<Self, SlickscreenError> {
        ffmpeg_next::init().map_err(|_e| SlickscreenError::FFmpegInitError)?;

        let time_reference = SlickscreenTime::new(std::time::Instant::now());

        let output_file_name = config
            .output_file
            .as_ref()
            .expect("no output file selected")
            .to_string();
        let worker =
            worker::Worker::new_consumer(move |control_receiver: SlickscreenMessageReceiver| {
                let mut ffmpeg_output =
                    ffmpeg_next::format::output(&output_file_name).expect("no output file");
                let stream_time_base = ffmpeg_next::util::rational::Rational::new(1, 1000000);

                let mut aac_encoder = ffmpeg_next::codec::encoder::audio::Audio(
                    ffmpeg_next::codec::encoder::encoder::Encoder(
                        ffmpeg_next::codec::Context::new(),
                    ),
                );
                aac_encoder.set_time_base(stream_time_base);
                aac_encoder.set_rate(48000);
                aac_encoder.set_format(ffmpeg_next::util::format::Sample::I16(
                    ffmpeg_next::util::format::sample::Type::Packed,
                ));
                aac_encoder.set_channels(2);
                aac_encoder
                    .set_channel_layout(ffmpeg_next::util::channel_layout::ChannelLayout::STEREO);
                let aac_codec = aac_encoder
                    .open_as(
                        ffmpeg_next::codec::encoder::find_by_name("libfdk_aac")
                            .expect("aac not found"),
                    )
                    .expect("unable to open aac codec");

                let mut aac_stream = ffmpeg_output
                    .add_stream(aac_codec.codec())
                    .expect("must be able to add aac stream");
                aac_stream.set_time_base(stream_time_base);
                aac_stream.set_parameters(ffmpeg_next::codec::Parameters::from(aac_codec));
                let aac_stream_index = aac_stream.index();

                let mut h264_encoder = ffmpeg_next::codec::encoder::video::Video(
                    ffmpeg_next::codec::encoder::encoder::Encoder(
                        ffmpeg_next::codec::Context::new(),
                    ),
                );
                h264_encoder.set_time_base(stream_time_base);
                h264_encoder.set_format(ffmpeg_next::util::format::pixel::Pixel::YUV420P);
                h264_encoder.set_width(1920);
                h264_encoder.set_height(1080);
                h264_encoder.set_gop(4096);
                h264_encoder.set_max_b_frames(0);
                h264_encoder.set_colorspace(ffmpeg_next::util::color::Space::BT709);
                h264_encoder.set_color_range(ffmpeg_next::util::color::Range::JPEG);
                h264_encoder.set_me_range(16);
                h264_encoder.set_qmin(10);
                h264_encoder.set_qmax(51);
                let mut h264_encoder_options = ffmpeg_next::Dictionary::new();
                h264_encoder_options.set("preset", "medium");
                h264_encoder_options.set("tune", "zerolatency");
                h264_encoder_options.set("level", "4.2");
                h264_encoder_options.set("profile", "high");
                h264_encoder_options.set("refs", "1");
                h264_encoder_options.set("crf", "15");
                h264_encoder_options.set("qdiff", "4");
                h264_encoder_options.set("qcompress", "0.6");
                h264_encoder_options.set("color_primaries", "bt709");
                h264_encoder_options.set("color_trc", "bt709");
                let h264_codec = h264_encoder
                    .open_as_with(
                        ffmpeg_next::codec::encoder::find_by_name("libx264")
                            .expect("h264 not found"),
                        h264_encoder_options,
                    )
                    .expect("unable to open h264 codec");
                let mut h264_stream = ffmpeg_output
                    .add_stream(h264_codec.codec())
                    .expect("could not add h264 stream");
                h264_stream.set_time_base(stream_time_base);
                h264_stream.set_parameters(ffmpeg_next::codec::Parameters::from(h264_codec));
                let h264_stream_index = h264_stream.index();

                if let Err(_) = ffmpeg_output.write_header() {
                    println!("Error while writing output file header");
                }

                ffmpeg_next::format::context::output::dump(&ffmpeg_output, 0, None);

                let aac_time_base = ffmpeg_output
                    .stream(aac_stream_index)
                    .expect("it was just added")
                    .time_base();
                let h264_time_base = ffmpeg_output
                    .stream(h264_stream_index)
                    .expect("it was just added")
                    .time_base();

                for msg in control_receiver.iter() {
                    match msg {
                        SlickscreenMessage::Quit => {
                            let _ = ffmpeg_output.write_trailer();
                            return;
                        }
                        SlickscreenMessage::Audio(packet) => {
                            let mut packet = packet;
                            packet.rescale_ts(stream_time_base, aac_time_base);
                            packet.set_stream(aac_stream_index);
                            if let Err(_) = packet.write_interleaved(&mut ffmpeg_output) {
                                println!("Error while writing audio packet");
                            }
                        }
                        SlickscreenMessage::Video(packet) => {
                            let mut packet = packet;
                            packet.rescale_ts(stream_time_base, h264_time_base);
                            packet.set_stream(h264_stream_index);
                            if let Err(_) = packet.write_interleaved(&mut ffmpeg_output) {
                                println!("Error while writing video packet");
                            }
                        }
                    }
                }
            });

        let control_sender = worker.control_sender();
        Ok(Self {
            worker,
            audio_recorder: AudioRecorder::new(time_reference, control_sender.clone())?,
            video_recorder: VideoRecorder::new(time_reference, control_sender.clone())?,
        })
    }

    pub fn stop(self) {
        let _ = self.audio_recorder.worker.stop();
        let _ = self.video_recorder.worker.stop();
        let _ = self.worker.stop();
    }
}
