use fluxr::{Curve, Program};

const WELL: &str = include_str!("../web/well.fluxr");
const REPLAY: &str = include_str!("../web/well.replay.json");

#[test]
fn both_frontends_and_compiled_roundtrip_preserve_replay() {
    let from_text = Program::compile_text(WELL).unwrap();
    let from_structured = Program::compile_structured(&from_text.structured()).unwrap();
    assert_eq!(from_text.encode(), from_structured.encode());
    let restored = Program::decode(&from_text.encode()).unwrap();
    assert_eq!(restored, from_text);
    assert_eq!(restored.replay(REPLAY).unwrap(), 9);
    assert_eq!(restored.sample_all(4200).collect::<Vec<_>>(), [-510]);
    assert!(restored.replay("[[41,1,0,509]]").is_err());
}

#[test]
fn malformed_and_incompatible_programs_fail_closed() {
    for source in [
        "v3 9\n1 2 0 0:2",
        "v1 9\n1 2 9 0:2",
        "v1 9\n1 2 0 1:2",
        "v1 9\n1 2 0 0:2 0:3",
        "v1 9\n1 2 0 0:2\n1 2 0 0:3",
    ] {
        assert!(Program::compile_text(source).is_err());
    }
    let mut bytes = Program::compile_text(WELL).unwrap().encode();
    for size in 0..bytes.len() {
        assert!(Program::decode(&bytes[..size]).is_err());
    }
    bytes.push(0);
    assert!(Program::decode(&bytes).is_err());
}

#[test]
fn migrated_runs_preserve_legacy_bytes_and_sample_order() {
    let source = "v1 15\n2 8 2 0:7 4:-1 12:5\n1 8 2 0:10 4:2 12:8\n3 8 2 0:4 4:-4 12:2";
    let original = Program::compile_text(source).unwrap();
    let encoded = original.encode();
    for version in [
        original,
        Program::decode(&encoded).unwrap(),
        Program::compile_structured(&Program::decode(&encoded).unwrap().structured()).unwrap(),
    ] {
        let migrated = version.compact_text();
        assert_eq!(migrated, "v2 15\n+ 1 3 8 2 -3 0:10 4:2 12:8");
        let restored = Program::compile_text(&migrated).unwrap();
        assert_eq!(restored.encode(), encoded);
        for tick in (0..30).chain([u32::MAX]) {
            assert!(
                restored
                    .sample_all(tick)
                    .eq(restored.addresses().map(|address| {
                        reference(Curve::Smooth, restored.keys(address).unwrap(), 15, tick)
                    }))
            );
        }
    }
    let mixed =
        Program::compile_text("v2 7\n+ 1 2 8 1 4294967295 0:-2147483648\n7 8 0 0:9").unwrap();
    assert_eq!(
        mixed.sample_all(0).collect::<Vec<_>>(),
        [i32::MIN, i32::MAX, 9]
    );
    for invalid in [
        "v2 9\n+ 0 0 1 0 0 0:0",
        "v2 9\n+ 0 129 1 0 0 0:0",
        "v2 9\n+ 65535 2 1 0 0 0:0",
        "v2 9\n+ 0 2 1 0 1 0:2147483647",
        "v2 9\n+ 0 2 1 0 -1 0:-2147483648",
        "v2 9\n+ 0 3 1 0 9223372036854775807 0:0",
        "v2 9\n+ 0 2 1 0 0 0:0\n1 1 0 0:0",
        "v1 9\n+ 0 2 1 0 0 0:0",
    ] {
        assert!(Program::compile_text(invalid).is_err(), "{invalid}");
    }
    let over_budget = (0..33).map(|i| format!(" {i}:0")).collect::<String>();
    assert!(Program::compile_text(&format!("v2 100\n+ 0 128 1 0 0{over_budget}")).is_err());
}

#[test]
fn generic_tracks_handle_signed_extremes_and_sorted_output() {
    let scene =
        Program::compile_text("v1 600000\n50 3 1 0:-2147483648 600000:2147483647\n1 1 0 0:15")
            .unwrap();
    assert_eq!(scene.addresses().collect::<Vec<_>>(), [(1, 1), (50, 3)]);
    assert_eq!(scene.sample_all(300000).collect::<Vec<_>>(), [15, -1]);
    assert_eq!(scene.sample((50, 3), 600000), Some(i32::MIN));
    assert_eq!(scene.sample((1, 9), 5), None);
}

fn reference(curve: Curve, keys: &[(u32, i32)], duration: u32, time_ms: u32) -> i32 {
    let t = time_ms % duration;
    for pair in keys.windows(2) {
        let [(start, from), (end, to)] = [pair[0], pair[1]];
        if t < end {
            let start_value = i128::from(from);
            let change = i128::from(to) - start_value;
            let elapsed = i128::from(t - start);
            let span = i128::from(end - start);
            let fixed = 65_536_i128;
            let adjustment = match curve {
                Curve::Hold => 0,
                Curve::Linear => change * elapsed / span,
                Curve::Smooth => {
                    let phase = elapsed * fixed / span;
                    let ease = phase * phase * (3 * fixed - 2 * phase) / (fixed * fixed);
                    change * ease / fixed
                }
            };
            return i32::try_from(start_value + adjustment).unwrap();
        }
    }
    keys.last().unwrap().1
}

#[test]
fn bound_tracks_match_signed_reference_across_full_periods() {
    const PERIOD: u32 = 600_000;
    let keys = [
        (0, i32::MIN),
        (100_000, -7),
        (330_000, i32::MAX),
        (599_997, -8),
    ];
    for curve in [Curve::Hold, Curve::Linear, Curve::Smooth] {
        let program = Program::compile_text(&format!(
            "v1 {PERIOD}\n1 9 {} 0:-2147483648 100000:-7 330000:2147483647 599997:-8",
            curve as u8,
        ))
        .unwrap();
        assert!(program.bind_track((1, 8)).is_none());
        let track = program.bind_track((1, 9)).unwrap();
        let restored = Program::decode(&program.encode()).unwrap();
        let restored_track = restored.bind_track((1, 9)).unwrap();
        for t in (0..=PERIOD).chain([PERIOD + 1, 2 * PERIOD, u32::MAX - 1, u32::MAX]) {
            let expected = reference(curve, &keys, PERIOD, t);
            assert_eq!(program.sample((1, 9), t), Some(expected));
            assert_eq!(track.sample(t), expected);
            assert_eq!(restored_track.sample(t), expected);
        }
        drop(program);
        let cloned = track.clone();
        drop(track);
        assert_eq!(cloned.sample(PERIOD), i32::MIN);
    }
}

#[test]
fn bound_track_handles_single_key_and_longer_key_tables() {
    let program = Program::compile_text(
        "v1 29\n1 1 0 0:-3\n1 2 1 0:-10 1:22 4:-100 5:13 9:0 10:2 23:7 29:100",
    )
    .unwrap();
    let constant = program.bind_track((1, 1)).unwrap();
    let varying = program.bind_track((1, 2)).unwrap();
    for t in (0..=58).chain([u32::MAX - 1, u32::MAX]) {
        assert_eq!(constant.sample(t), -3);
        assert_eq!(
            varying.sample(t),
            reference(Curve::Linear, program.keys((1, 2)).unwrap(), 29, t)
        );
    }
}
