use fluxr::{Program, RuleProgram, telemetry};
use std::{error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
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
    let [command, source, input] = args else {
        return Err("usage: fluxr-cli [--stats] <test|encode|encode-structured|emit-gate> <source.fluxr|source.flux|source.json|source.rule.json|compiled.fxr> <replay.json|output.fxr>".into());
    };
    let bytes = fs::read(source)?;
    if command == "emit-gate" {
        let source = std::str::from_utf8(&bytes)?;
        fs::write(
            input,
            fluxr::Gate::compile_text(source)?.rust_function("bench_gate")?,
        )?;
        return Ok(());
    }
    if command == "encode-structured" {
        let text = std::str::from_utf8(&bytes)?;
        let module = if text.trim_start().starts_with('v') {
            fluxr::structured::Module::compile_compact(text)?
        } else {
            fluxr::structured::Module::compile_text(text)?
        };
        fs::write(input, module.encode())?;
        return Ok(());
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
            _ => Err("command must be test or encode".into()),
        };
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
        _ => return Err("command must be test or encode".into()),
    }
    Ok(())
}
