mod migration;
mod structured_cli;
mod unified_cli;

use fluxr::structured::Module;
use fluxr::{Program, RuleProgram, telemetry};
use migration::{Legacy, Options};
use std::{error::Error, fs, io::Read, path::Path};

// Match the released reader limits before allocating the whole input. FSM's
// canonical JSON payload has a larger budget than compact source and a six-byte
// envelope. The parsers remain authoritative for format and semantic checks.
const MAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_CANONICAL_BYTES: usize = 1024 * 1024;
const MAX_FXH_BYTES: usize = 512 * 1024;

fn read_limited(
    reader: impl Read,
    limit: usize,
    message: &'static str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(message.into());
    }
    Ok(bytes)
}

fn read_structured_file(path: &str, allow_legacy_json: bool) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut file = fs::File::open(path)?;
    let mut prefix = Vec::new();
    file.by_ref().take(4).read_to_end(&mut prefix)?;
    let (limit, message) = if prefix == b"FSM\0" {
        (
            MAX_CANONICAL_BYTES + 6,
            "structured program exceeds canonical byte budget",
        )
    } else if allow_legacy_json {
        // Leading whitespace may precede a legacy JSON object. Its parser
        // retains the 1 MiB limit; compact text still keeps its 256 KiB limit.
        (
            MAX_CANONICAL_BYTES,
            "structured program exceeds canonical byte budget",
        )
    } else {
        (MAX_SOURCE_BYTES, "program exceeds byte budget")
    };
    read_limited(prefix.as_slice().chain(file), limit, message)
}

fn read_replay_file(path: &str) -> Result<String, Box<dyn Error>> {
    Ok(String::from_utf8(read_limited(
        fs::File::open(path)?,
        MAX_SOURCE_BYTES,
        "program exceeds byte budget",
    )?)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    // A compiled CLI and every generated application retain the same runtime.
    let _runtime = fluxr::runtime::global();
    #[cfg(not(target_arch = "wasm32"))]
    let _ = telemetry::configure_from_env();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let report = args.first().is_some_and(|arg| arg == "--stats");
    if report {
        args.remove(0);
    }
    let result = run(&args);
    if report {
        eprintln!("{}", telemetry::snapshot());
    }
    result
}

fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if unified_cli::accepts(args) {
        return unified_cli::run(args);
    }
    let (receipt, args) = match args {
        [flag, rest @ ..] if flag == "--receipt" => (true, rest),
        _ => (false, args),
    };
    let (replay, args) = match args {
        [flag, path, rest @ ..] if flag == "--replay" => (Some(path.as_str()), rest),
        _ => (None, args),
    };
    let options = Options { receipt, replay };
    if (receipt || replay.is_some())
        && !args.first().is_some_and(|command| {
            matches!(
                command.as_str(),
                "migrate-timeline"
                    | "migrate-structured"
                    | "migrate-rule"
                    | "migrate-structured-hir"
                    | "migrate-rule-hir"
            )
        })
    {
        return Err("--receipt and --replay require a migration command".into());
    }
    if let [command, args @ ..] = args
        && command == "structured"
    {
        return structured_cli::run(args);
    }
    if let [command, source, export, output] = args
        && command == "emit-native-structured"
    {
        let module = read_module(&read_structured_file(source, false)?, false)?;
        fs::write(output, module.emit_native_rust(export)?)?;
        return Ok(());
    }
    let [command, source, input] = args else {
        return Err("usage: fluxr [--stats] <test|encode|encode-structured|encode-agent|encode-wordcode|expand-wordcode|project-agent|emit-gate|restore-hir> <source> <replay-or-output>\n       fluxr emit-native-structured <source> <export> <output>\n       fluxr [--stats] [--receipt] [--replay <probes>] <migrate-timeline|migrate-structured|migrate-rule|migrate-structured-hir|migrate-rule-hir> <source> <output>\n       fluxr [--stats] structured <command> ...".into());
    };
    let bytes = if matches!(
        command.as_str(),
        "migrate-structured-hir" | "migrate-rule-hir" | "restore-hir"
    ) {
        read_limited(
            fs::File::open(source)?,
            MAX_FXH_BYTES,
            "FXH input exceeds byte limit",
        )?
    } else if matches!(command.as_str(), "migrate-structured" | "project-agent") {
        read_structured_file(source, true)?
    } else {
        read_limited(
            fs::File::open(source)?,
            MAX_SOURCE_BYTES,
            "program exceeds byte budget",
        )?
    };
    if command == "restore-hir" {
        return migration::restore_hir(&bytes, input);
    }
    if command == "emit-gate" {
        let source = std::str::from_utf8(&bytes)?;
        fs::write(
            input,
            fluxr::Gate::compile_text(source)?.rust_function("bench_gate")?,
        )?;
        return Ok(());
    }
    match command.as_str() {
        "encode-wordcode" | "expand-wordcode" => {
            let dictionary = fluxr::structured::wordcode::Dictionary::core();
            let compiled = fluxr::structured::wordcode::compile(
                std::str::from_utf8(&bytes)?,
                &dictionary,
                fluxr::structured::wordcode::ExpansionLimits::default(),
            )?;
            return if command == "encode-wordcode" {
                Ok(fs::write(input, compiled.module.encode())?)
            } else {
                Ok(fs::write(input, compiled.expanded_agent)?)
            };
        }
        "encode-agent" => {
            let module = Module::compile_agent(std::str::from_utf8(&bytes)?)?;
            return Ok(fs::write(input, module.encode())?);
        }
        "project-agent" => {
            let module = read_module(&bytes, true)?;
            return Ok(fs::write(input, module.agent_text()?)?);
        }
        "encode-structured" => {
            let module = Module::compile_compact(std::str::from_utf8(&bytes)?)?;
            return Ok(fs::write(input, module.encode())?);
        }
        "migrate-structured" => {
            return Legacy::Structured(read_module(&bytes, true)?).write(&bytes, input, options);
        }
        "migrate-structured-hir" => {
            return Legacy::Structured(read_module(&bytes, true)?)
                .write_hir(&bytes, input, options);
        }
        _ => (),
    }

    let extension = Path::new(source).extension().and_then(|ext| ext.to_str());
    let rules = match extension {
        Some("fluxr" | "flux") if std::str::from_utf8(&bytes)?.trim_start().starts_with('r') => {
            Some(RuleProgram::compile_text(std::str::from_utf8(&bytes)?)?)
        }
        Some("json") if source.ends_with(".rule.json") => Some(RuleProgram::compile_structured(
            std::str::from_utf8(&bytes)?,
        )?),
        Some("fxr") if bytes.starts_with(b"FXD\0") => Some(RuleProgram::decode(&bytes)?),
        _ => None,
    };
    if let Some(rules) = rules {
        return match command.as_str() {
            "test" => {
                println!(
                    "{} rule assertions passed",
                    rules.replay(&read_replay_file(input)?)?
                );
                Ok(())
            }
            "encode" => {
                fs::write(input, rules.encode())?;
                Ok(())
            }
            "migrate-rule" => {
                Legacy::Rule(rules).write(&bytes, input, options)?;
                Ok(())
            }
            "migrate-rule-hir" => Legacy::Rule(rules).write_hir(&bytes, input, options),
            _ => Err("command must be test, encode, or migrate-rule".into()),
        };
    }
    if matches!(command.as_str(), "migrate-rule" | "migrate-rule-hir") {
        return Err(format!("{command} requires rule text, .rule.json, or an FXD artifact").into());
    }
    let program = match extension {
        Some("fluxr" | "flux") => Program::compile_text(std::str::from_utf8(&bytes)?)?,
        Some("json") => Program::compile_structured(std::str::from_utf8(&bytes)?)?,
        Some("fxr") => Program::decode(&bytes)?,
        _ => return Err("source extension must be fluxr, flux, json, or fxr".into()),
    };
    match command.as_str() {
        "test" => println!(
            "{} scenario assertions passed",
            program.replay(&read_replay_file(input)?)?
        ),
        "encode" => fs::write(input, program.encode())?,
        "migrate-timeline" => Legacy::Timeline(program).write(&bytes, input, options)?,
        _ => return Err("command must be test, encode, or migrate-timeline".into()),
    }
    Ok(())
}

fn read_module(bytes: &[u8], allow_legacy_json: bool) -> Result<Module, Box<dyn Error>> {
    if bytes.starts_with(b"FSM\0") {
        return Ok(Module::decode(bytes)?);
    }
    let source = std::str::from_utf8(bytes)?;
    if allow_legacy_json && source.trim_start().starts_with('{') {
        Ok(Module::compile_legacy_json(source)?)
    } else {
        Ok(Module::compile_compact(source)?)
    }
}

#[cfg(test)]
mod input_limit_tests {
    use super::read_limited;
    use std::io::{self, Read};

    struct Endless {
        read: usize,
    }

    impl Read for Endless {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            bytes.fill(b'x');
            self.read += bytes.len();
            Ok(bytes.len())
        }
    }

    #[test]
    fn read_limit_accepts_exact_size_and_reads_only_one_overflow_byte() {
        assert_eq!(read_limited(&b"1234"[..], 4, "oversize").unwrap(), b"1234");
        let mut source = Endless { read: 0 };
        assert_eq!(
            read_limited(&mut source, 4, "oversize")
                .unwrap_err()
                .to_string(),
            "oversize"
        );
        assert_eq!(source.read, 5);
    }
}
