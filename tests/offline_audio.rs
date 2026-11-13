use fluxr::cpu_compute::{
    ABI, Cancellation, LibraryManifest, MemoryBudget, NUMERIC_PROFILE, ScalarLibrary,
};
use fluxr::offline_audio::{Composition, Note, PPQN, RenderLimits, Tempo, Track, render};
use sha2::{Digest, Sha256};
fn constant() -> ScalarLibrary {
    let source = "v2 (pub sample (phase age velocity:f32) f32 = velocity)";
    let manifest = LibraryManifest {
        abi: ABI.into(),
        numeric_profile: NUMERIC_PROFILE.into(),
        source_sha256: format!("{:x}", Sha256::digest(source)),
        package: "test.instrument".into(),
        version: 1,
        exports: std::collections::BTreeMap::from([("sample".into(), 3)]),
        capabilities: vec!["compute.cpu".into()],
    };
    ScalarLibrary::load(&serde_json::to_vec(&manifest).unwrap(), source).unwrap()
}
fn composition() -> Composition {
    Composition {
        tempo: Tempo {
            beats_per_minute_milli: 120000,
            beats_per_bar: 4,
            beat_unit: 4,
        },
        sample_rate: 8000,
        duration_ticks: PPQN,
        tracks: vec![Track {
            instrument: 0,
            notes: vec![Note {
                start_tick: PPQN / 4,
                duration_ticks: PPQN / 4,
                frequency_hz: 200.,
                velocity: 0.5,
            }],
            gain: 1.,
            pan_start: 0.,
            pan_end: 0.,
        }],
        delay_ticks: PPQN / 4,
        delay_feedback: 0.,
        lowpass: 1.,
    }
}
#[test]
fn exact_sample_boundaries_and_repeated_render_are_stable() {
    let libraries = [constant()];
    let memory = MemoryBudget::new(100000);
    let cancel = Cancellation::default();
    let first = render(
        &composition(),
        &libraries,
        RenderLimits::default(),
        &memory,
        &cancel,
    )
    .unwrap();
    assert_eq!(first.samples.shape(), &[4000, 2]);
    assert_eq!(first.report.calls, 1000);
    for frame in 0..4000 {
        let expected = if (1000..2000).contains(&frame) {
            0.25
        } else {
            0.
        };
        assert_eq!(
            &first.samples.values()[frame * 2..frame * 2 + 2],
            &[expected, expected]
        );
    }
    let first_bytes = first.samples.encode();
    drop(first);
    let second = render(
        &composition(),
        &libraries,
        RenderLimits::default(),
        &memory,
        &cancel,
    )
    .unwrap();
    assert_eq!(first_bytes, second.samples.encode());
    assert!(!second.report.real_time_qualified);
    assert_eq!(second.report.active_voice_peak, 1);
}
#[test]
fn render_rejects_malformed_events_limits_and_unavailable_instruments() {
    let libraries = [constant()];
    let memory = MemoryBudget::new(100000);
    let cancel = Cancellation::default();
    for mutation in 0..8 {
        let mut c = composition();
        let mut limits = RenderLimits::default();
        match mutation {
            0 => c.tracks[0].instrument = 1,
            1 => c.tracks[0].notes[0].duration_ticks = u64::MAX,
            2 => c.tracks[0].notes[0].frequency_hz = f32::NAN,
            3 => c.delay_feedback = 1.,
            4 => limits.max_frames = 10,
            5 => limits.max_calls = 10,
            6 => limits.max_events = 0,
            _ => limits.block_frames = 0,
        }
        assert!(render(&c, &libraries, limits, &memory, &cancel).is_err());
        assert_eq!(memory.live_bytes(), 0);
    }
    let mut c = composition();
    let note = c.tracks[0].notes[0].clone();
    c.tracks[0].notes.push(note);
    let limits = RenderLimits {
        max_voices: 1,
        ..RenderLimits::default()
    };
    assert!(render(&c, &libraries, limits, &memory, &cancel).is_err());
    let tiny = MemoryBudget::new(8);
    assert!(
        render(
            &composition(),
            &libraries,
            RenderLimits::default(),
            &tiny,
            &cancel
        )
        .is_err()
    );
    assert_eq!(tiny.live_bytes(), 0);
    cancel.cancel();
    assert!(
        render(
            &composition(),
            &libraries,
            RenderLimits::default(),
            &memory,
            &cancel
        )
        .is_err()
    );
}
#[test]
fn rational_timing_has_no_accumulated_rounding_drift() {
    let tempo = Tempo {
        beats_per_minute_milli: 123456,
        beats_per_bar: 7,
        beat_unit: 8,
    };
    assert_eq!(
        tempo.sample_at(1000000, 48000).unwrap(),
        ((1000000u128 * 48000 * 60000) / (123456 * 960)) as u64
    );
    assert!(
        Tempo {
            beats_per_minute_milli: 0,
            ..tempo
        }
        .sample_at(0, 48000)
        .is_err()
    );
    assert!(tempo.sample_at(0, 0).is_err());
}
#[test]
fn independently_packaged_instruments_compile_and_sound_different() {
    let sources = [
        (
            include_bytes!("../examples/native/instruments/triangle/manifest.json").as_slice(),
            include_str!("../examples/native/instruments/triangle/program.flux"),
        ),
        (
            include_bytes!("../examples/native/instruments/pluck/manifest.json").as_slice(),
            include_str!("../examples/native/instruments/pluck/program.flux"),
        ),
        (
            include_bytes!("../examples/native/instruments/bell/manifest.json").as_slice(),
            include_str!("../examples/native/instruments/bell/program.flux"),
        ),
        (
            include_bytes!("../examples/native/instruments/noise/manifest.json").as_slice(),
            include_str!("../examples/native/instruments/noise/program.flux"),
        ),
    ];
    let mut results = Vec::new();
    for (manifest, source) in sources {
        let library = ScalarLibrary::load(manifest, source).unwrap();
        let mut session = library.session(
            fluxr::structured::Limits::default(),
            1,
            Cancellation::default(),
        );
        results.push(
            session
                .call("sample", &[0.23, 0.17, 0.7])
                .unwrap()
                .to_bits(),
        );
    }
    results.sort_unstable();
    results.dedup();
    assert_eq!(results.len(), 4);
}
