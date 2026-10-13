use crate::*;
use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

#[test]
fn audio_configuration_round_trips_three_independent_rates() {
    let parameters = AudioParameters {
        rf_bandwidth_hz: 12_500,
        output_sample_rate_hz: 44_100,
        stereo: false,
        cw_tone_hz: 800,
    };
    let spec = ChannelSpec {
        id: 71,
        center_offset_hz: -50_000,
        channel_sample_rate_hz: 200_000,
        audio: Some(parameters),
        kind: PipelineKind::Audio,
        demod_mode: Some(DemodMode::Fm),
        meteor_profile: None,
    };
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(json["channel_sample_rate_hz"], 200_000);
    assert_eq!(json["audio"]["rf_bandwidth_hz"], 12_500);
    assert_eq!(json["audio"]["output_sample_rate_hz"], 44_100);
    assert_eq!(json["audio"]["stereo"], false);
    assert_eq!(json["audio"]["cw_tone_hz"], 800);
    assert!(json.get("bandwidth_hz").is_none());
    assert_eq!(serde_json::from_value::<ChannelSpec>(json).unwrap(), spec);
    let mut codec = MessageCodec::for_commands();
    let mut bytes = BytesMut::new();
    for command in [
        ClientCommand::Subscribe { channel: spec },
        ClientCommand::SetDemodMode {
            channel_id: 71,
            mode: DemodMode::Fm,
            audio: Some(parameters),
        },
        ClientCommand::Retune {
            channel_id: 71,
            center_offset_hz: 25_000,
            channel_sample_rate_hz: 256_000,
            audio: Some(AudioConfiguration {
                mode: DemodMode::Wfm,
                parameters: AudioParameters::for_mode(DemodMode::Wfm),
            }),
        },
    ] {
        codec.encode(command.clone(), &mut bytes).unwrap();
        assert_eq!(codec.decode(&mut bytes).unwrap(), Some(command));
    }
}

#[test]
fn authentication_command_serializes_as_a_plain_string_and_redacts_debug() {
    let command = ClientCommand::Authenticate {
        token: AuthToken::new("field-test-secret"),
    };
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(json["Authenticate"]["token"], "field-test-secret");
    assert!(!format!("{command:?}").contains("field-test-secret"));
}

#[test]
fn legacy_json_bandwidth_alias_is_explicitly_a_channel_clock() {
    let old =
        r#"{"id":71,"center_offset_hz":0,"bandwidth_hz":200000,"kind":"Audio","demod_mode":"Fm"}"#;
    let spec: ChannelSpec = serde_json::from_str(old).unwrap();
    assert_eq!(spec.channel_sample_rate_hz, 200_000);
    assert_eq!(spec.audio, None);
    let command: ClientCommand = serde_json::from_str(
        r#"{"Retune":{"channel_id":71,"center_offset_hz":0,"bandwidth_hz":200000}}"#,
    )
    .unwrap();
    assert_eq!(
        command,
        ClientCommand::Retune {
            channel_id: 71,
            center_offset_hz: 0,
            channel_sample_rate_hz: 200_000,
            audio: None
        }
    );
}

#[test]
fn audio_frame_round_trip_keeps_rf_capture_and_pcm_clock_distinct() {
    let mut rf_clock = CaptureClock::new(81, 700, 2_048_003, None);
    rf_clock.sample_stride = 10;
    let pcm_clock = CaptureClock::new(3, 117, 48_000, None);
    let mut codec = MessageCodec::for_data();
    let mut bytes = BytesMut::new();
    codec
        .encode(
            ServerEvent::Audio(AudioFrame {
                clock: rf_clock,
                audio_clock: pcm_clock,
                sequence: 4,
                channel_id: 71,
                sample_rate_hz: 48_000,
                channels: 2,
                squelch: SquelchState {
                    db: -40.0,
                    revision: 2,
                },
                samples: vec![0.2, -0.1, 0.4, 0.3],
            }),
            &mut bytes,
        )
        .unwrap();
    let Some(ServerEvent::Audio(frame)) = codec.decode(&mut bytes).unwrap() else {
        panic!("missing PCM frame")
    };
    assert_eq!(frame.clock, rf_clock);
    assert_eq!(frame.audio_clock, pcm_clock);
    assert_eq!(frame.channels, 2);
    assert_eq!(frame.samples, [0.2, -0.1, 0.4, 0.3]);
}
