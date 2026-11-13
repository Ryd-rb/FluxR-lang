//! The greeting bytes are computed by the checked FluxR program, not this host.
use fluxr::structured::{Invocation, Limits, Module, Value};
use std::io::Write;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = Module::compile_compact(include_str!("hello-agent.fluxr"))?;
    let module = Module::decode(&module.encode())?;
    let Value::List(bytes) = module.run(Invocation {
        export: "hello",
        inputs: &[],
        arguments: &[],
        limits: Limits::default(),
    })?
    else {
        return Err("expected FluxR byte list".into());
    };
    let greeting: Vec<u8> = bytes
        .iter()
        .map(|v| match v {
            Value::I64(v) => u8::try_from(*v).map_err(|_| "invalid byte"),
            _ => Err("invalid byte type"),
        })
        .collect::<Result<_, _>>()?;
    std::io::stdout().write_all(&greeting)?;
    Ok(())
}
