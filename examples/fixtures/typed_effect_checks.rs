//! Compare the source-level specification with four public runtime routes.
use super::{
    Replay, SCAN,
    list_effect_checks::list,
    typed_effect_spec::{Expr::*, Spec},
};
use fluxr::structured::{Binding, Limits, Module, Type, Value};

const PREPARED: bool = cfg!(all(
    feature = "typed-jit",
    any(
        target_os = "windows",
        all(target_os = "linux", target_env = "gnu")
    ),
    target_arch = "x86_64"
));
const INPUT: &str = "v2 (inputs samples:[f32]) (pub pick () u64 () (ret (minidx (input samples))))";
const MIXED: &str =
    "v2 (inputs samples:[f32]) (pub pick (extra:[f32]) u64 () (ret (minidx (input samples))))";

struct Checker {
    replay: Replay<'static>,
    spec: Spec,
}

impl Checker {
    fn new(source: &str, spec: Spec, prepared: bool) -> Self {
        let module = Module::compile_compact(source).unwrap();
        assert_eq!(module.prepare_native("pick"), prepared, "{source}");
        let grants: Vec<_> = spec
            .inputs
            .iter()
            .map(|&lists| {
                let ty = (0..lists).fold(Type::F32, |ty, _| Type::List(Box::new(ty)));
                Binding {
                    name: "samples".into(),
                    ty,
                }
            })
            .collect();
        Self {
            replay: Replay::new(module, "pick", &[], &grants),
            spec,
        }
    }

    fn check(&mut self, inputs: &[Value], arguments: &[Value], limits: Limits) {
        let expected = self.spec.run(inputs, arguments, limits);
        self.replay.check_call(inputs, arguments, limits, expected);
    }

    fn grid(&mut self, inputs: &[Value], arguments: &[Value], length: usize) {
        // Include all nearby thresholds for short lists and bounded windows
        // at both ends of long lists, plus the maximum representable limits.
        let mut edges: Vec<_> = (0..=8)
            .chain(length.saturating_sub(1)..=length + 8)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        for &cells in &edges {
            for &fuel in &edges {
                for frames in [0, 1] {
                    self.check(
                        inputs,
                        arguments,
                        Limits {
                            instructions: fuel as u64,
                            cells,
                            frames,
                        },
                    );
                }
            }
        }
        self.check(
            inputs,
            arguments,
            Limits {
                instructions: u64::MAX,
                cells: usize::MAX,
                frames: usize::MAX,
            },
        );
    }
}

fn floats(bits: &[u32]) -> Value {
    list(bits.iter().copied().map(Value::F32))
}

fn fuzz_scan(argument: &mut Checker, input: &mut Checker, index: &mut Checker) {
    let mut state = 0x505b_7a5d_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for sample in 0..256 {
        let len = (next() % 65) as usize;
        let mut values: Vec<_> = (0..len).map(|_| Value::F32(next())).collect();
        if len > 0 && sample % 4 == 0 {
            values[next() as usize % len] = Value::Unit;
        }
        let call = [list(values)];
        let limits = match sample % 4 {
            1 => Limits::default(),
            _ => Limits {
                instructions: u64::from(next()) % (len as u64 + 7),
                cells: next() as usize % (len + 8),
                frames: usize::from(next() % 8 != 0),
            },
        };
        argument.check(&[], &call, limits);
        input.check(&call, &[], limits);
        index.check(&[], &call, limits);
    }
}

pub(crate) fn check_typed_effects() -> usize {
    let mut argument = Checker::new(
        SCAN,
        Spec {
            inputs: &[],
            arguments: &[1],
            locals: 0,
            body: Scan(&Argument),
        },
        PREPARED,
    );
    let mut input = Checker::new(
        INPUT,
        Spec {
            inputs: &[1],
            arguments: &[],
            locals: 0,
            body: Scan(&Input),
        },
        PREPARED,
    );
    let mut mixed = Checker::new(
        MIXED,
        Spec {
            inputs: &[1],
            arguments: &[1],
            locals: 0,
            body: Scan(&Input),
        },
        false,
    );
    let mut index = Checker::new(
        "v2 (pub pick (values:[f32]) f32 () (ret (at values (minidx values))))",
        Spec {
            inputs: &[],
            arguments: &[1],
            locals: 0,
            body: At(&Argument, &Scan(&Argument)),
        },
        false,
    );
    let mut uninitialized = Checker::new(
        "v2 (pub pick () u64 (values:[f32]) (ret (minidx values)))",
        Spec {
            inputs: &[],
            arguments: &[],
            locals: 1,
            body: Scan(&Uninitialized),
        },
        false,
    );
    let mut constructed = Checker::new(
        "v2 (pub pick () u64 () (ret (minidx (list f32 0x00000001f 0x80000001f))))",
        Spec {
            inputs: &[],
            arguments: &[],
            locals: 0,
            body: Scan(&Literal(&[1, 0x8000_0001])),
        },
        false,
    );
    let mut nested = Checker::new(
        "v2 (pub pick (batches:[[f32]]) u64 () (ret (minidx (at batches 0u))))",
        Spec {
            inputs: &[],
            arguments: &[2],
            locals: 0,
            body: Scan(&At(&Argument, &Zero)),
        },
        false,
    );

    let malformed = [
        Value::Unit,
        Value::Bool(false),
        Value::I64(0),
        Value::U64(0),
        Value::Record([Value::F32(0)].into()),
        Value::Variant(0, None),
        floats(&[]),
    ];
    for bad in malformed
        .iter()
        .filter(|bad| !matches!(bad, Value::List(_)))
    {
        let call = std::slice::from_ref(bad);
        argument.grid(&[], call, 0);
        input.grid(call, &[], 0);
    }

    let patterns = [
        0xff80_0000,
        0x8000_0001,
        0x8000_0000,
        0,
        1,
        0x7f80_0000,
        0x7f80_0001,
        0x7fc0_0031,
        0xff80_0040,
    ];
    for len in 0..=3 {
        for mut code in 0..patterns.len().pow(len) {
            let bits: Vec<_> = (0..len)
                .map(|_| {
                    let value = patterns[code % patterns.len()];
                    code /= patterns.len();
                    value
                })
                .collect();
            let values = [floats(&bits)];
            let exact = Limits {
                instructions: u64::from(len) + 3,
                cells: len as usize + 4,
                frames: 1,
            };
            argument.check(&[], &values, exact);
            input.check(
                &values,
                &[],
                Limits {
                    cells: exact.cells - 1,
                    ..exact
                },
            );
        }
    }
    for len in [0, 1, 3, 16, 256] {
        let mut bits: Vec<_> = (0..len).map(|i| patterns[i % patterns.len()]).collect();
        for &first in &[0x7f80_0001, 1][..if len == 0 { 1 } else { 2 }] {
            if let Some(bits) = bits.first_mut() {
                *bits = first;
            }
            let values = [floats(&bits)];
            argument.grid(&[], &values, len);
            input.grid(&values, &[], len);
            index.grid(&[], &values, len);
        }
        let positions: &[usize] = match len {
            0 => &[],
            1 => &[0],
            _ => &[0, len / 2, len - 1],
        };
        for &position in positions {
            for bad in &malformed {
                let mut values: Vec<_> = bits.iter().copied().map(Value::F32).collect();
                values[position] = bad.clone();
                let call = [list(values)];
                // Compete the bad tag against the exact first affordable cell.
                for cells in [position + 1, position + 2, len + 5] {
                    let limits = Limits {
                        instructions: 0,
                        cells,
                        frames: 0,
                    };
                    argument.check(&[], &call, limits);
                    input.check(&call, &[], limits);
                }
            }
        }
    }

    let valid = [floats(&[1])];
    let bad_child = [list([Value::Unit])];
    let bad_root = [Value::F32(0)];
    for (inputs, arguments) in [
        (&valid, &bad_child),
        (&bad_child, &bad_root),
        (&bad_root, &bad_child),
    ] {
        mixed.grid(inputs, arguments, 2);
    }
    for (inputs, arguments) in [
        (&[][..], &[][..]),
        (&bad_root, &bad_root),
        (&[][..], &[Value::Unit, Value::Unit]),
        (&[Value::Unit, Value::Unit], &[][..]),
    ] {
        let zero = Limits {
            instructions: 0,
            cells: 0,
            frames: 0,
        };
        argument.check(inputs, arguments, zero);
        input.check(inputs, arguments, zero);
        mixed.check(inputs, arguments, zero);
    }
    uninitialized.grid(&[], &[], 0);
    constructed.grid(&[], &[], 2);
    for values in [
        floats(&[]),
        list([floats(&[])]),
        list([floats(&[1, 0])]),
        list([Value::F32(0)]),
        list([bad_child[0].clone()]),
        list([floats(&[1]), bad_child[0].clone()]),
    ] {
        nested.grid(&[], &[values], 3);
    }
    fuzz_scan(&mut argument, &mut input, &mut index);

    for source in [
        "v1 (pub pick (values:[f32]) u64 () (ret (minidx values)))",
        "v2 (pub pick (values:[u64]) u64 () (ret (minidx values)))",
        "v2 (pub pick (values:[f32]) f32 () (ret (minidx values)))",
        "v2 (pub pick (value:f32) u64 () (ret (minidx value)))",
    ] {
        assert!(Module::compile_compact(source).is_err(), "{source}");
    }
    [
        argument,
        input,
        mixed,
        index,
        uninitialized,
        constructed,
        nested,
    ]
    .into_iter()
    .map(|checker| checker.replay.count)
    .sum()
}
