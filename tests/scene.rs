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
        "v2 9\n1 2 0 0:2",
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
