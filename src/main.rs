mod migration;
mod structured_cli;

use fluxr::structured::Module;
use fluxr::{Program, RuleProgram, telemetry};
use migration::{Legacy, Options};
use std::{error::Error, fs, io::Read, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
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
        let module = read_module(&fs::read(source)?, false)?;
        fs::write(output, module.emit_native_rust(export)?)?;
        return Ok(());
    }
    let [command, source, input] = args else {
        return Err("usage: fluxr [--stats] <test|encode|encode-structured|emit-gate|restore-hir> <source> <replay-or-output>\n       fluxr emit-native-structured <source> <export> <output>\n       fluxr [--stats] [--receipt] [--replay <probes>] <migrate-timeline|migrate-structured|migrate-rule|migrate-structured-hir|migrate-rule-hir> <source> <output>\n       fluxr [--stats] structured <command> ...".into());
    };
    let bytes = if matches!(
        command.as_str(),
        "migrate-structured-hir" | "migrate-rule-hir" | "restore-hir"
    ) {
        const LIMIT: usize = 512 * 1024;
        let mut bytes = Vec::new();
        fs::File::open(source)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > LIMIT {
            return Err("FXH input exceeds byte limit".into());
        }
        bytes
    } else {
        fs::read(source)?
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
                    rules.replay(&fs::read_to_string(input)?)?
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
            program.replay(&fs::read_to_string(input)?)?
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
