use super::*;

use ffmpeg_next::codec::encoder;
use ffmpeg_next::codec::encoder::audio::Audio;
use ffmpeg_next::codec::encoder::encoder::Encoder;
use ffmpeg_next::codec::Context;
use ffmpeg_next::util::channel_layout::ChannelLayout;
use ffmpeg_next::util::format::sample as ffmpeg_sample;
use ffmpeg_next::util::frame::Audio as AudioFrame;

use crate::worker::WorkerControlMessage;

pub(super) enum AudioRecorderMessage {
    Quit,
    RawAudioPacket(i64, ffmpeg_next::util::frame::Audio),
}

impl From<worker::WorkerControlMessage> for AudioRecorderMessage {
    fn from(msg: WorkerControlMessage) -> Self {
        match msg {
            worker::WorkerControlMessage::Quit => AudioRecorderMessage::Quit,
        }
    }
}

pub(super) struct AudioRecorder {
    pub worker: worker::Worker<AudioRecorderMessage>,
    stream: cpal::Stream,
}

impl AudioRecorder {
    pub fn new(
        time_reference: SlickscreenTime,
        slickscreen_message_sender: SlickscreenMessageSender,
    ) -> Result<Self, SlickscreenError> {
        let default_device = cpal::default_host()
            .default_output_device()
            .ok_or(SlickscreenError::Unexpected)?;
        let sample_rate: usize = 48000;
        let channel_count: usize = 2;
        let sample_count: usize = sample_rate / 100;
        let config = cpal::StreamConfig {
            channels: channel_count as cpal::ChannelCount,
            sample_rate: cpal::SampleRate(sample_rate as u32),
            buffer_size: cpal::BufferSize::Fixed(sample_count as cpal::FrameCount),
        };

        let encoder_format = ffmpeg_sample::Sample::I16(ffmpeg_sample::Type::Packed);
        let encoder_channel_layout = ChannelLayout::STEREO;
        let encoder_context = Context::new();
        let mut encoder = Audio(Encoder(encoder_context));
        encoder.set_time_base(ffmpeg_next::util::rational::Rational::new(1, 1000000));
        encoder.set_rate(sample_rate as i32);
        encoder.set_format(encoder_format);
        encoder.set_channels(channel_count as i32);
        encoder.set_channel_layout(encoder_channel_layout);
        // find_by_name("libfdk_aac") - interleaved S16
        // find_by_name("libopus") - interleaved S16
        let mut encoder = encoder
            .open_as(
                encoder::find_by_name("libfdk_aac")
                    .ok_or(SlickscreenError::AudioEncoderNotFound)?,
            )
            .map_err(|_e| SlickscreenError::AudioEncoderNotFound)?;

        let worker = worker::Worker::new(
            slickscreen_message_sender,
            move |worker_sender: SlickscreenMessageSender,
                  control_receiver: crossbeam::channel::Receiver<AudioRecorderMessage>| {
                for msg in control_receiver.iter() {
                    match msg {
                        AudioRecorderMessage::Quit => {
                            return;
                        }
                        AudioRecorderMessage::RawAudioPacket(_pts, frame) => {
                            if let Err(e) = encoder.send_frame(&frame) {
                                println!("Error while encoding audio frame: {}", e);
                                return;
                            }

                            let mut packet = ffmpeg_next::Packet::empty();
                            while let Ok(_) = encoder.receive_packet(&mut packet) {
                                if let Err(e) =
                                    worker_sender.send(SlickscreenMessage::Audio(packet.clone()))
                                {
                                    println!("Unable to send encoded audio packet. Audio encoder worker exiting. - {}", e);
                                    return;
                                }
                            }
                        }
                    }
                }
            },
        );

        let worker_sender = worker.control_sender();
        let stream = default_device
            .build_input_stream_raw(
                &config,
                cpal::SampleFormat::I16,
                move |data: &cpal::Data, _input_info: &cpal::InputCallbackInfo| {
                    let now = time_reference.pts_now();

                    let mut frame = AudioFrame::new(
                        encoder_format,
                        data.len() / channel_count,
                        encoder_channel_layout,
                    );
                    frame.set_pts(Some(now));

                    frame.data_mut(0)[0..data.bytes().len()].copy_from_slice(data.bytes());
                    if let Err(e) =
                        worker_sender.send(AudioRecorderMessage::RawAudioPacket(now, frame))
                    {
                        println!("Audio recorder worker thread appears to be dead. - {:?}", e);
                    }
                },
                move |err| {
                    eprintln!("an error occurred on stream: {:?}", err);
                },
            )
            .map_err(|e| SlickscreenError::AudioCaptureError(e.to_string()))?;

        stream.play().map_err(|_e| SlickscreenError::Unexpected)?;

        Ok(Self { worker, stream })
    }
}
