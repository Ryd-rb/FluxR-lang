//! Bounded WK1 process protocol. A host owns process admission and hard deadlines.
//! Native code never crosses a process boundary: the prepared worker executes it.
use fluxr::structured::{self, Invocation, Limits, Value, Workspace, wordcode};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::io::{self, Read, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_FRAME: usize = 512 * 1024;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_CALLS: u64 = 65_536;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("host clock after epoch")
        .as_nanos() as u64
}

fn frame(input: &mut impl Read) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let mut header = [0; 4];
    match input.read(&mut header[..1])? {
        0 => return Ok(None),
        1 => input.read_exact(&mut header[1..])?,
        _ => unreachable!(),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err("WK1 worker frame limit".into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

fn reply(output: &mut impl Write, value: Json) -> Result<(), Box<dyn Error>> {
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > MAX_FRAME {
        return Err("WK1 worker response limit".into());
    }
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Init {
    protocol: u32,
    source: String,
    source_sha256: String,
    dictionary_sha256: String,
    dictionary_json: Option<String>,
    export: String,
    generation: u64,
    mode: String,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Call {
        sequence: u64,
        inputs: Vec<Value>,
        arguments: Vec<Value>,
        limits: Limits,
    },
    Close,
}

fn serve() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let init: Init = serde_json::from_slice(&frame(&mut input)?.ok_or("missing init")?)?;
    let initialization_started_unix_ns = timestamp();
    if init.protocol != 1 || init.source.len() > MAX_SOURCE {
        return Err("WK1 worker protocol or source limit".into());
    }
    if !matches!(init.mode.as_str(), "portable" | "prepared") {
        return Err("WK1 worker unknown execution mode".into());
    }
    let dictionary_start = Instant::now();
    let dictionary = match init.dictionary_json {
        Some(ref text) => wordcode::Dictionary::from_json(text)?,
        None => wordcode::Dictionary::core(),
    };
    let dictionary_seconds = dictionary_start.elapsed().as_secs_f64();
    if init.dictionary_sha256 != dictionary.identity()
        || init.source_sha256 != digest(init.source.as_bytes())
    {
        return Err("WK1 worker immutable source/dictionary mismatch".into());
    }
    let start = Instant::now();
    let compiled = wordcode::compile(
        &init.source,
        &dictionary,
        wordcode::ExpansionLimits::default(),
    )?;
    let expansion_seconds = start.elapsed().as_secs_f64();
    let module = compiled.module;
    let module_sha256 = digest(&module.encode());
    // Preparing is confined to this process; portable workers never call it.
    let start = Instant::now();
    let preparation_started_unix_ns = timestamp();
    let prepared = if init.mode == "prepared" {
        Some(module.prepare_call(&init.export)?)
    } else {
        None
    };
    let preparation_seconds = start.elapsed().as_secs_f64();
    reply(
        &mut output,
        json!({
            "protocol": 1, "kind": "ready", "generation": init.generation,
            "source_sha256": init.source_sha256, "dictionary_sha256": dictionary.identity(),
            "module_sha256": module_sha256, "expanded_sha256": digest(compiled.expanded_agent.as_bytes()),
            "export": init.export, "mode": init.mode, "typed_jit_feature": cfg!(feature = "typed-jit"),
            "dictionary_seconds": dictionary_seconds,
            "initialization_started_unix_ns": initialization_started_unix_ns,
            "preparation_started_unix_ns": preparation_started_unix_ns,
            "ready_unix_ns": timestamp(),
            "expansion_seconds": expansion_seconds, "preparation_seconds": preparation_seconds,
            "max_calls": MAX_CALLS, "max_frame_bytes": MAX_FRAME
        }),
    )?;
    let mut workspace = Workspace::default();
    let mut previous = 0;
    while let Some(bytes) = frame(&mut input)? {
        let request: Request = serde_json::from_slice(&bytes)?;
        let Request::Call {
            sequence,
            inputs,
            arguments,
            limits,
        } = request
        else {
            break;
        };
        if sequence != previous + 1 || sequence > MAX_CALLS {
            return Err("WK1 worker sequence/call bound".into());
        }
        previous = sequence;
        let start = Instant::now();
        let (result, route) = if let Some(prepared) = &prepared {
            prepared.run_with_route(&inputs, &arguments, limits, &mut workspace)
        } else {
            (
                module.run_portable_with(
                    Invocation {
                        export: &init.export,
                        inputs: &inputs,
                        arguments: &arguments,
                        limits,
                    },
                    &mut workspace,
                ),
                structured::ExecutionRoute::Portable,
            )
        };
        let execution_seconds = start.elapsed().as_secs_f64();
        let (value, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        };
        reply(
            &mut output,
            json!({
                "protocol": 1, "kind": "result", "generation": init.generation,
                "sequence": sequence, "module_sha256": module_sha256,
                "route": format!("{route:?}"), "value": value, "error": error,
                "execution_seconds": execution_seconds
            }),
        )?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "worker" => serve(),
        [command] if command == "dictionary" => {
            let dictionary = wordcode::Dictionary::core();
            println!(
                "{}",
                json!({"profile":"wk1", "binding":dictionary.header(),
                "dictionary_sha256":dictionary.identity(), "dictionary_json":dictionary.to_json()})
            );
            Ok(())
        }
        [command, path, rest @ ..] if command == "expand" && rest.len() <= 1 => {
            let mut source = String::new();
            std::fs::File::open(path)?
                .take((MAX_SOURCE + 1) as u64)
                .read_to_string(&mut source)?;
            if source.len() > MAX_SOURCE {
                return Err("WK1 source limit".into());
            }
            let dictionary = if let Some(path) = rest.first() {
                let mut text = String::new();
                std::fs::File::open(path)?
                    .take(65_537)
                    .read_to_string(&mut text)?;
                wordcode::Dictionary::from_json(&text)?
            } else {
                wordcode::Dictionary::core()
            };
            let compiled =
                wordcode::compile(&source, &dictionary, wordcode::ExpansionLimits::default())?;
            println!(
                "{}",
                json!({"profile":"wk1", "dictionary_sha256":dictionary.identity(),
                "source_sha256":digest(source.as_bytes()), "expanded":compiled.expanded_agent,
                "module_sha256":digest(&compiled.module.encode())})
            );
            Ok(())
        }
        _ => Err("usage: wordcode_worker <worker|dictionary|expand SOURCE [DICTIONARY]>".into()),
    }
}
