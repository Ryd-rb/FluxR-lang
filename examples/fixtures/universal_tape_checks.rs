use super::{FUEL, Replay, list_effect_checks::list};
use fluxr::structured::{Binding, Invocation, Limits, Module, Type, Value};
use std::collections::BTreeMap;

const SOURCE: &str = include_str!("universal_tape.flux");
type Rule = (u64, u64, bool);

fn record(fields: impl IntoIterator<Item = Value>) -> Value {
    Value::Record(fields.into_iter().collect())
}

// The reference uses an absolute position and sparse memory, independently
// of the two stacks in the FluxR program.
struct Machine {
    state: u64,
    position: i64,
    first: i64,
    last: i64,
    tape: BTreeMap<i64, u64>,
}

impl Machine {
    fn new(word: &[u64]) -> Self {
        Self {
            state: 1,
            position: 0,
            first: 0,
            last: word.len().saturating_sub(1) as i64,
            tape: word
                .iter()
                .enumerate()
                .map(|(i, &symbol)| (i as i64, symbol))
                .collect(),
        }
    }

    fn step(&mut self, width: usize, rules: &[Rule]) {
        if self.state == 0 {
            return;
        }
        let read = self.symbol(self.position) as usize;
        let (next, write, right) = rules[(self.state as usize - 1) * width + read];
        self.tape.insert(self.position, write);
        self.state = next;
        self.position += if right { 1 } else { -1 };
        self.first = self.first.min(self.position);
        self.last = self.last.max(self.position);
    }

    fn symbol(&self, position: i64) -> u64 {
        self.tape.get(&position).copied().unwrap_or(0)
    }

    fn snapshot(&self) -> Value {
        let cell = |position| Value::U64(self.symbol(position));
        record([
            Value::U64(self.state),
            cell(self.position),
            list((self.first..self.position).map(cell)),
            list((self.position + 1..=self.last).rev().map(cell)),
        ])
    }
}

pub(crate) fn check_universal_tape() -> usize {
    let module = Module::compile_compact(SOURCE).unwrap();
    assert!(!module.prepare_native("advance"));
    let grants = [
        Binding {
            name: "alphabet".into(),
            ty: Type::U64,
        },
        Binding {
            name: "rules".into(),
            ty: Type::List(Box::new(Type::Record("Rule".into()))),
        },
    ];
    let mut replay = Replay::new(module.clone(), "advance", &[], &grants);
    let limits = Limits::default();
    let busy_beaver = [(2, 1, true), (2, 1, false), (1, 1, false), (0, 1, true)];
    let walk_left = [(1, 0, false)];
    let replace = [(0, 0, true), (1, 1, true), (0, 1, false)];
    for (width, rules, word, count, final_state) in [
        (2, &busy_beaver[..], &[][..], 8, 0),
        (1, &walk_left[..], &[][..], 24, 1),
        (3, &replace[..], &[1, 2, 1][..], 4, 0),
    ] {
        let inputs = [
            Value::U64(width as u64),
            list(rules.iter().map(|&(next, write, right)| {
                record([Value::U64(next), Value::U64(write), Value::Bool(right)])
            })),
        ];
        let mut oracle = Machine::new(word);
        let initial = oracle.snapshot();
        let mut checkpoint = initial.clone();
        for steps in 0..=count {
            let expected = oracle.snapshot();
            replay.check_call(
                &inputs,
                &[initial.clone(), Value::U64(steps)],
                limits,
                Ok(expected.clone()),
            );
            if steps != 0 {
                let args = [checkpoint.clone(), Value::U64(1)];
                // A failed invocation can be retried from the last explicit
                // checkpoint. The successful step uses the same host snapshot.
                let call = |limits| Invocation {
                    export: "advance",
                    inputs: &inputs,
                    arguments: &args,
                    limits,
                };
                let starved = Limits {
                    instructions: 10,
                    ..limits
                };
                replay.check_call(&inputs, &args, starved, Err(FUEL));
                checkpoint = module.run(call(limits)).unwrap();
                assert_eq!(checkpoint, expected);
            }
            if steps != count {
                oracle.step(width, rules);
            }
        }
        assert_eq!(oracle.state, final_state);
        if width == 2 {
            assert_eq!(
                oracle.tape.values().filter(|&&symbol| symbol == 1).count(),
                4
            );
        }
        assert_eq!(initial, Machine::new(word).snapshot());
    }
    replay.count
}
