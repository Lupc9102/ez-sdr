fn configured_audio_test_state() -> DaemonState {
    let state = test_state();
    // The shared fixture's constructor says 2 MS/s, while its synthetic source
    // actually supplies 2.048 MS/s. Initialize geometry from the applied clock.
    let applied_rate = state.hardware.status().sample_rate_hz;
    assert_eq!(applied_rate, 2_048_000);
    state
        .channelizer
        .lock()
        .set_wideband_rate_hz(applied_rate)
        .unwrap();
    state
}

#[test]
fn audio_configuration_validates_all_three_rates_before_allocation() {
    let state = configured_audio_test_state();
    for parameters in [
        AudioParameters {
            rf_bandwidth_hz: 0,
            output_sample_rate_hz: 48_000,
        },
        AudioParameters {
            rf_bandwidth_hz: 180_000,
            output_sample_rate_hz: 48_000,
        },
        AudioParameters {
            rf_bandwidth_hz: 25_000,
            output_sample_rate_hz: 7_999,
        },
        AudioParameters {
            rf_bandwidth_hz: 25_000,
            output_sample_rate_hz: u32::MAX,
        },
    ] {
        let mut spec = audio_spec(1, 0);
        spec.audio = Some(parameters);
        assert!(state.subscribe(spec).is_err());
        assert!(state.active_channels().is_empty());
        assert_eq!(state.channelizer.lock().channel_count(), 0);
    }
    let mut spectrum = audio_spec(2, 0);
    spectrum.kind = PipelineKind::Spectrum;
    spectrum.audio = Some(AudioParameters::for_mode(DemodMode::Fm));
    assert!(state.subscribe(spectrum).is_err());
    assert!(state.active_channels().is_empty());
}

#[test]
fn audio_mode_rf_width_and_pcm_rate_retune_atomically_in_both_directions() {
    let state = configured_audio_test_state();
    let mut spec = audio_spec(1, 0);
    spec.audio = Some(AudioParameters {
        rf_bandwidth_hz: 25_000,
        output_sample_rate_hz: 44_100,
    });
    let (_subscription, _) = state.subscribe(spec.clone()).unwrap();
    let internal_id = state
        .channels
        .lock()
        .get(&1)
        .unwrap()
        .internal_channel_id
        .unwrap();
    for (mode, channel_rate, rf_width, pcm_rate, actual_rate) in [
        (DemodMode::Wfm, 256_000, 180_000, 48_000, 256_000),
        (DemodMode::Fm, 200_000, 12_500, 8_000, 204_800),
        (DemodMode::Am, 96_000, 15_000, 44_100, 97_523),
    ] {
        let configuration = AudioConfiguration {
            mode,
            parameters: AudioParameters {
                rf_bandwidth_hz: rf_width,
                output_sample_rate_hz: pcm_rate,
            },
        };
        state
            .retune_configured(1, 10_000, channel_rate, Some(configuration))
            .unwrap();
        let applied = state
            .active_channels()
            .into_iter()
            .find(|s| s.id == 1)
            .unwrap();
        assert_eq!(applied.channel_sample_rate_hz, channel_rate);
        assert_eq!(applied.audio, Some(configuration.parameters));
        assert_eq!(applied.demod_mode, Some(mode));
        assert_eq!(
            state.channelizer.lock().channel_output_rate_hz(internal_id),
            Some(actual_rate)
        );
        let channels = state.channels.lock();
        let active = channels.get(&1).unwrap();
        assert_eq!(active.internal_channel_id, Some(internal_id));
        let ActivePipeline::Audio(pipeline) = &active.pipeline else {
            panic!("audio pipeline replaced")
        };
        assert_eq!(pipeline.lock().mode(), mode);
    }
    let before = state.active_channels();
    let before_rate = state.channelizer.lock().channel_output_rate_hz(internal_id);
    assert!(state
        .retune_configured(
            1,
            100_000,
            200_000,
            Some(AudioConfiguration {
                mode: DemodMode::Wfm,
                parameters: AudioParameters::for_mode(DemodMode::Wfm),
            })
        )
        .is_err());
    assert_eq!(state.active_channels(), before);
    assert_eq!(
        state.channelizer.lock().channel_output_rate_hz(internal_id),
        before_rate
    );
    assert!(state
        .configure_audio(
            1,
            DemodMode::Fm,
            Some(AudioParameters {
                rf_bandwidth_hz: 0,
                output_sample_rate_hz: 48_000
            })
        )
        .is_err());
    assert_eq!(state.active_channels(), before);
    let channels = state.channels.lock();
    let ActivePipeline::Audio(pipeline) = &channels.get(&1).unwrap().pipeline else {
        unreachable!()
    };
    assert_eq!(pipeline.lock().mode(), DemodMode::Am);
}
