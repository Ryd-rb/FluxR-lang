//! The public command interface uses the same runtime as embedded applications.
//! Historical encoders remain compatibility readers in the legacy CLI module.
use fluxr::runtime::{
    Command, ControlRequest, ControlResponse, ImportKind, Runtime, RuntimeError,
    handle_control_request, import_source,
};
use fluxr::structured::{Limits, Value};
use std::{
    error::Error,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_SOURCE: usize = 8 * 1024 * 1024;
const MAX_WIRE: usize = 2 * 1024 * 1024;

pub const USAGE: &str = "FluxR — one language, one injectable runtime\n\
usage: fluxr-cli version\n\
       fluxr-cli run SOURCE EXPORT [ARGUMENTS_JSON] [INPUTS_JSON]\n\
       fluxr-cli test-suite CATALOG [OPTIONS]\n\
       fluxr-cli test SOURCE REPLAY\n\
       fluxr-cli serve\n\
       fluxr-cli install ADDRESS NAME <new|REVISION> SOURCE\n\
       fluxr-cli call ADDRESS NAME EXPORT [ARGUMENTS_JSON] [INPUTS_JSON]\n\
       fluxr-cli inspect ADDRESS NAME\n\
       fluxr-cli list ADDRESS\n\
       fluxr-cli remove ADDRESS NAME REVISION\n\
       fluxr-cli import <fa1|compact|fsm|wk1> SOURCE OUTPUT\n\
       fluxr-cli build PROJECT OUTPUT --target-dir TARGET --logs LOGS [OPTIONS]\n\n\
Current source starts with `fluxr 1`. Compilation retains the runtime.\n\
`serve` accepts bounded ControlRequest JSON lines over inherited stdin/stdout.\n\
TCP client commands require FLUXR_CONTROL_TOKEN and a loopback ADDRESS.\n\
Historical encoding/migration commands remain compatibility adapters.";

pub fn accepts(args: &[String]) -> bool {
    args.is_empty()
        || args.first().is_some_and(|x| {
            matches!(
                x.as_str(),
                "help"
                    | "--help"
                    | "-h"
                    | "version"
                    | "--version"
                    | "run"
                    | "test-suite"
                    | "serve"
                    | "install"
                    | "call"
                    | "inspect"
                    | "list"
                    | "remove"
                    | "import"
                    | "build"
                    | "internal-emit-aot"
            )
        })
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("input exceeds its byte limit".into());
    }
    Ok(bytes)
}

fn source(path: &str) -> Result<String, Box<dyn Error>> {
    Ok(String::from_utf8(read_bounded(
        fs::File::open(path)?,
        MAX_SOURCE,
    )?)?)
}

fn values(text: Option<&String>) -> Result<Vec<Value>, Box<dyn Error>> {
    match text {
        None => Ok(Vec::new()),
        Some(text) if text.len() <= MAX_WIRE => Ok(serde_json::from_str(text)?),
        Some(_) => Err("argument JSON exceeds its byte limit".into()),
    }
}

fn write_json(out: &mut impl Write, value: &impl serde::Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()
}

fn failure(code: &str, message: impl Into<String>) -> ControlResponse {
    ControlResponse {
        version: 1,
        ok: false,
        result: None,
        error: Some(RuntimeError::new(code, message)),
    }
}

fn report(response: ControlResponse) -> Result<(), Box<dyn Error>> {
    write_json(&mut io::stdout().lock(), &response)?;
    if response.ok {
        Ok(())
    } else {
        Err("runtime request was rejected; see the structured response".into())
    }
}

fn revision(text: &str) -> Result<u64, Box<dyn Error>> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("revision must be an unsigned decimal integer".into());
    }
    Ok(text.parse()?)
}

fn remote(address: &str, command: Command) -> Result<(), Box<dyn Error>> {
    let address: SocketAddr = address.parse()?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("control address must be a loopback IP and nonzero port".into());
    }
    let token = std::env::var("FLUXR_CONTROL_TOKEN")
        .map_err(|_| "FLUXR_CONTROL_TOKEN is required for TCP control")?;
    if token.is_empty() {
        return Err("FLUXR_CONTROL_TOKEN must not be empty".into());
    }
    let request = ControlRequest {
        version: 1,
        token,
        command,
    };
    let mut payload = serde_json::to_vec(&request)?;
    if payload.len() + 1 > MAX_WIRE {
        return Err("control request exceeds its byte limit".into());
    }
    payload.push(b'\n');
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&payload)?;
    let mut line = Vec::new();
    let size = BufReader::new(stream)
        .take(MAX_WIRE as u64 + 1)
        .read_until(b'\n', &mut line)?;
    if size == 0 || size > MAX_WIRE || !line.ends_with(b"\n") {
        return Err("control response is missing, truncated, or oversized".into());
    }
    let response: ControlResponse = serde_json::from_slice(&line)?;
    if response.version != 1 {
        return Err("unsupported control response version".into());
    }
    report(response)
}

fn serve(
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<(), Box<dyn Error>> {
    let token = std::env::var("FLUXR_CONTROL_TOKEN").unwrap_or_default();
    let mut input = input;
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_WIRE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_WIRE || !line.ends_with(b"\n") {
            write_json(
                &mut output,
                &failure("input_limit", "control line is oversized or incomplete"),
            )?;
            return Err("control input framing failed".into());
        }
        let response = match serde_json::from_slice::<ControlRequest>(&line) {
            Ok(request) => handle_control_request(runtime, request, &token),
            Err(error) => failure("invalid_request", error.to_string()),
        };
        write_json(&mut output, &response)?;
    }
}

fn build_arguments(
    args: &[String],
    default_compiler: impl FnOnce() -> io::Result<PathBuf>,
) -> io::Result<Vec<OsString>> {
    // The suffix after the first literal separator belongs solely to libtest.
    // It must neither suppress frontend defaults nor receive frontend options.
    let separator = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    let explicit_compiler = args[..separator]
        .iter()
        .any(|arg| arg == "--compiler" || arg.starts_with("--compiler="));
    let mut forwarded: Vec<OsString> = args.iter().map(OsString::from).collect();
    if !explicit_compiler {
        forwarded.splice(
            separator..separator,
            [
                OsString::from("--compiler"),
                default_compiler()?.into_os_string(),
            ],
        );
    }
    Ok(forwarded)
}

fn build(args: &[String]) -> Result<(), Box<dyn Error>> {
    // This command is a build-time convenience. Application execution never
    // invokes Python. The exact source-local implementation is the sole builder.
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/fluxr.py");
    if !tool.is_file() {
        return Err("build frontend is not installed beside this compiler; use the source checkout's tools/fluxr.py".into());
    }
    let python = std::env::var_os("FLUXR_PYTHON").unwrap_or_else(|| "python3".into());
    let mut command = std::process::Command::new(python);
    command
        .arg("-B")
        .arg(tool)
        .arg("build")
        .args(build_arguments(args, std::env::current_exe)?);
    let status = command.status()?;
    if !status.success() {
        return Err(format!("build frontend exited with {status}").into());
    }
    Ok(())
}

fn emit_aot(path: &str, output: &str) -> Result<(), Box<dyn Error>> {
    use sha2::{Digest, Sha256};
    let text = source(path)?;
    let document = fluxr::runtime::Document::parse(&text)?;
    let module = document.compile()?;
    let mut emitted = String::from("// Generated from checked FluxR by the shared compiler.\n");
    let mut bindings = String::new();
    let mut exports = Vec::new();
    for (index, name) in module.export_signatures().keys().enumerate() {
        match module.emit_native_binding(name) {
            Ok(code) => {
                emitted.push_str(&format!(
                    "#[allow(dead_code)]\nmod export_{index} {{\n{code}\n}}\n"
                ));
                bindings.push_str(&format!(
                    "    export_{index}::bind(runtime, name, revision)?;\n"
                ));
                exports.push(serde_json::json!({"name":name,"status":"aot"}));
            }
            Err(error) => exports.push(serde_json::json!({
                "name":name,"status":"vm","reason":error.to_string()
            })),
        }
        if emitted.len() > 16 * 1024 * 1024 {
            return Err("generated AOT code exceeds its byte limit".into());
        }
    }
    emitted.push_str("pub fn bind(runtime: &fluxr::runtime::Runtime, name: &str, revision: u64) -> Result<(), fluxr::runtime::RuntimeError> {\n");
    emitted.push_str(&bindings);
    emitted.push_str("    let _ = (runtime, name, revision);\n    Ok(())\n}\n");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(emitted.as_bytes())?;
    file.sync_all()?;
    write_json(
        &mut io::stdout().lock(),
        &serde_json::json!({
            "language_version":fluxr::runtime::LANGUAGE_VERSION,
            "source_sha256":format!("{:x}",Sha256::digest(text.as_bytes())),
            "exports":exports,"output":output
        }),
    )?;
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(command) = args.first() else {
        println!("{USAGE}");
        return Ok(());
    };
    let rest = &args[1..];
    match (command.as_str(), rest) {
        ("help" | "--help" | "-h", []) => println!("{USAGE}"),
        ("version" | "--version", []) => {
            write_json(
                &mut io::stdout().lock(),
                &serde_json::json!({
                    "language": "FluxR", "language_version": fluxr::runtime::LANGUAGE_VERSION,
                    "runtime": "embedded", "source_header": "fluxr 1"
                }),
            )?;
        }
        ("run", [path, export, tail @ ..]) if tail.len() <= 2 => {
            let runtime = fluxr::runtime::global();
            let install = runtime.execute(Command::Install {
                name: "main".into(),
                expected_revision: None,
                source: source(path)?,
            });
            if !install.ok {
                return report(install);
            }
            return report(runtime.execute(Command::Call {
                name: "main".into(),
                export: export.clone(),
                inputs: values(tail.get(1))?,
                arguments: values(tail.first())?,
                limits: Limits::default(),
            }));
        }
        ("test-suite", rest) => {
            if !fluxr::testing::run_cli(rest)? {
                return Err("FluxR test suite failed; see the test results".into());
            }
        }
        ("serve", []) => {
            return serve(
                &fluxr::runtime::global(),
                io::stdin().lock(),
                io::stdout().lock(),
            );
        }
        ("install", [address, name, expected, path]) => {
            return remote(
                address,
                Command::Install {
                    name: name.clone(),
                    expected_revision: if expected == "new" {
                        None
                    } else {
                        Some(revision(expected)?)
                    },
                    source: source(path)?,
                },
            );
        }
        ("call", [address, name, export, tail @ ..]) if tail.len() <= 2 => {
            return remote(
                address,
                Command::Call {
                    name: name.clone(),
                    export: export.clone(),
                    inputs: values(tail.get(1))?,
                    arguments: values(tail.first())?,
                    limits: Limits::default(),
                },
            );
        }
        ("inspect", [address, name]) => {
            return remote(address, Command::Inspect { name: name.clone() });
        }
        ("list", [address]) => return remote(address, Command::List),
        ("remove", [address, name, expected]) => {
            return remote(
                address,
                Command::Remove {
                    name: name.clone(),
                    expected_revision: revision(expected)?,
                },
            );
        }
        ("import", [kind, path, output]) => {
            let kind = match kind.as_str() {
                "fa1" => ImportKind::Agent,
                "compact" => ImportKind::Compact,
                "fsm" => ImportKind::Fsm,
                "wk1" => ImportKind::Wordcode,
                _ => return Err("import format must be fa1, compact, fsm, or wk1".into()),
            };
            let bytes = read_bounded(fs::File::open(path)?, MAX_SOURCE)?;
            let text = import_source(kind, &bytes)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        ("build", rest) if !rest.is_empty() => return build(rest),
        ("internal-emit-aot", [path, output]) => return emit_aot(path, output),
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_reader_stops_after_one_overflow_byte() {
        assert_eq!(read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
        assert!(read_bounded(&b"12345"[..], 4).is_err());
    }

    #[test]
    fn revision_parser_does_not_accept_signs_or_overflow() {
        assert_eq!(revision("9").unwrap(), 9);
        for text in ["", "-1", "+1", " 1", "18446744073709551616"] {
            assert!(revision(text).is_err(), "{text}");
        }
    }

    #[test]
    fn client_rejects_non_loopback_before_reading_token_or_connecting() {
        assert!(
            remote("192.0.2.1:1234", Command::List)
                .unwrap_err()
                .to_string()
                .contains("loopback")
        );
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn os_strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn implicit_compiler_precedes_libtest_separator_and_preserves_arguments() {
        let args = strings(&[
            "project.json",
            "output",
            "--test",
            "--",
            "name with spaces",
            "--exact",
        ]);
        let actual = build_arguments(&args, || Ok(PathBuf::from("selected compiler"))).unwrap();
        assert_eq!(
            actual,
            os_strings(&[
                "project.json",
                "output",
                "--test",
                "--compiler",
                "selected compiler",
                "--",
                "name with spaces",
                "--exact",
            ])
        );
        let args = strings(&["project.json", "output"]);
        assert_eq!(
            build_arguments(&args, || Ok(PathBuf::from("selected compiler"))).unwrap(),
            os_strings(&["project.json", "output", "--compiler", "selected compiler"]),
        );
    }

    #[test]
    fn explicit_compiler_in_frontend_prefix_does_not_resolve_a_default() {
        for args in [
            strings(&[
                "project.json",
                "output",
                "--compiler",
                "explicit compiler",
                "--test",
                "--",
                "filter",
            ]),
            strings(&[
                "project.json",
                "output",
                "--compiler=explicit compiler",
                "--test",
                "--",
                "filter",
            ]),
        ] {
            let actual = build_arguments(&args, || {
                panic!("explicit compiler must not resolve a default")
            })
            .unwrap();
            assert_eq!(actual, args.iter().map(OsString::from).collect::<Vec<_>>());
        }
    }

    #[test]
    fn suffix_compiler_spellings_are_preserved_and_do_not_suppress_defaults() {
        for suffix in [
            vec!["--compiler", "literal value"],
            vec!["--compiler=literal value"],
            vec!["--", "--compiler"],
        ] {
            let mut args = strings(&["project.json", "output", "--test", "--"]);
            args.extend(strings(&suffix));
            let actual = build_arguments(&args, || Ok(PathBuf::from("selected compiler"))).unwrap();
            let mut expected = os_strings(&[
                "project.json",
                "output",
                "--test",
                "--compiler",
                "selected compiler",
                "--",
            ]);
            expected.extend(os_strings(&suffix));
            assert_eq!(actual, expected);
        }
    }
}
