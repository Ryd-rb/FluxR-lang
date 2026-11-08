use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};
use serde::Deserialize;
use serde_json::json;
use std::{
    error::Error,
    io::{self, BufRead, Write},
};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Inspect,
    Tick {
        count: i64,
    },
    Inject {
        expected_revision: u64,
        source: String,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("provide the initial module source")?;
    let source = std::fs::read_to_string(path)?;
    let grants = [Binding {
        name: "bonus".into(),
        ty: Type::I64,
    }];
    let mut slot = ModuleSlot::new(Module::compile_text(&source)?, &grants)?;
    let mut workspace = Workspace::default();
    let mut state = Value::Record([Value::I64(10)].into());
    let inputs = [Value::I64(3)];
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<(), Box<dyn Error>> {
            match serde_json::from_str::<Request>(&line?)? {
                Request::Inspect => {}
                Request::Tick { count } => {
                    let arguments = [state.clone(), Value::I64(count)];
                    let invocation = Invocation {
                        export: "tick",
                        inputs: &inputs,
                        arguments: &arguments,
                        limits: Limits::default(),
                    };
                    state = slot.run_with(invocation, &mut workspace)?;
                }
                Request::Inject {
                    expected_revision,
                    source,
                } => {
                    let candidate = Module::compile_text(&source)?;
                    let arguments = [state.clone(), Value::I64(6)];
                    slot.inject(
                        expected_revision,
                        candidate,
                        Some(Invocation {
                            export: "tick",
                            inputs: &inputs,
                            arguments: &arguments,
                            limits: Limits::default(),
                        }),
                    )?;
                }
            }
            Ok(())
        })();
        let receipt = json!({
            "pid": std::process::id(), "revision": slot.revision(), "state": state,
            "error": result.err().map(|e| e.to_string()),
        });
        writeln!(out, "{receipt}")?;
        out.flush()?;
    }
    Ok(())
}
