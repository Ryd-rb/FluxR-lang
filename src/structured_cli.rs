use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{self, BufRead, Read, Write},
    path::{Component, Path},
    sync::Arc,
};

const USAGE: &str = "usage: fluxr-cli structured call <manifest> <export> [integer ...]\n       fluxr-cli structured lines <manifest> <adapter> <table> <init> <step> [integer ...]\n       fluxr-cli structured series <manifest> <time|trace> <start> <step> <inspect> <warmup> <samples> [integer ...]";
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
const NATIVE_USAGE: &str = "\n       fluxr-cli structured app <application.json> [--mcp <capture-directory>] [--peer <role> <address>] [integer ...]\n       fluxr-cli structured service <service.json>";
const MAX_TEXT: usize = 2 * 1024 * 1024;
const MAX_LINE: u64 = 4096;

fn append(path: &Path, text: &mut String) -> io::Result<()> {
    let remaining = MAX_TEXT - text.len();
    let n = File::open(path)?
        .take(remaining as u64)
        .read_to_string(text)?;
    if n == remaining {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source too large",
        ));
    }
    text.push('\n');
    Ok(())
}

fn manifest(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut entries = String::new();
    append(path, &mut entries)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let mut source = String::new();
    for entry in entries
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let entry = Path::new(entry);
        if !entry
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err("manifest entries must be relative paths within its directory".into());
        }
        let start = source.len();
        append(&root.join(entry), &mut source)?;
        // A module that is also opened independently can retain its version line.
        if start > 0
            && let Some((line, _)) = source[start..].split_once('\n')
            && matches!(line.trim(), "v1" | "v2")
            && source
                .split_once('\n')
                .is_some_and(|(first, _)| first.trim() == line.trim())
        {
            source.drain(start..start + line.len() + 1);
        }
    }
    Ok(source)
}

fn table(text: &str) -> Result<BTreeMap<&str, i64>, Box<dyn Error>> {
    let mut table = BTreeMap::new();
    for (i, row) in text.lines().enumerate() {
        let name = row.split('\t').next().unwrap_or_default();
        if name.is_empty() || name.trim() != name || name.len() as u64 >= MAX_LINE {
            return Err("invalid name in command table".into());
        }
        if table.insert(name, i as i64).is_some() {
            return Err(format!("duplicate command {name:?}").into());
        }
    }
    Ok(table)
}

fn run_export(
    module: &Module,
    workspace: &mut Workspace,
    export: &str,
    arguments: &[Value],
) -> Result<Value, fluxr::Error> {
    module.run_with(
        Invocation {
            export,
            inputs: &[],
            arguments,
            limits: Limits::default(),
        },
        workspace,
    )
}

struct LineProgram<'a> {
    module: Module,
    table: BTreeMap<&'a str, i64>,
    init: &'a str,
    step: &'a str,
}

impl LineProgram<'_> {
    fn replay(
        &self,
        args: &[Value],
        mut input: impl BufRead,
        mut output: impl Write,
    ) -> Result<(), Box<dyn Error>> {
        let mut workspace = Workspace::default();
        let Value::Variant(1, Some(state)) =
            run_export(&self.module, &mut workspace, self.init, args)?
        else {
            return Err("initialization refused or returned an invalid result".into());
        };
        writeln!(output, "{state:?}")?;
        let mut state = Arc::unwrap_or_clone(state);
        let mut line = String::new();
        loop {
            line.clear();
            let read = (&mut input).take(MAX_LINE).read_line(&mut line)?;
            if read == 0 {
                break;
            }
            if read as u64 == MAX_LINE {
                return Err("input line too long".into());
            }
            let name = line.trim();
            let code = *self
                .table
                .get(name)
                .ok_or_else(|| format!("unknown order {name:?}"))?;
            let result = run_export(
                &self.module,
                &mut workspace,
                self.step,
                &[state, Value::I64(code)],
            )?;
            let Value::Record(fields) = result else {
                return Err("expected a state and receipt record".into());
            };
            let [next, receipt] = fields.as_ref() else {
                return Err("expected exactly two step result fields".into());
            };
            writeln!(output, "{receipt:?}")?;
            state = next.clone();
        }
        Ok(())
    }
}

fn integers(args: &[String]) -> Result<Vec<Value>, std::num::ParseIntError> {
    args.iter().map(|arg| arg.parse().map(Value::I64)).collect()
}

pub fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args {
        #[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
        [command, path] if command == "service" => {
            fluxr::native_runtime::application_service(Path::new(path))?;
        }
        #[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
        [command, path, args @ ..] if command == "app" => {
            let mut args = args;
            let mut captures = None;
            let mut peer = None;
            loop {
                match args {
                    [flag, directory, rest @ ..] if flag == "--mcp" && captures.is_none() => {
                        captures = Some(Path::new(directory));
                        args = rest;
                    }
                    [flag, role, address, rest @ ..] if flag == "--peer" && peer.is_none() => {
                        peer = Some(fluxr::native_runtime::PeerRequest::parse(role, address)?);
                        args = rest;
                    }
                    _ => break,
                }
            }
            let app = fluxr::native_runtime::Application::load(Path::new(path), |path| {
                Ok(Module::compile_compact(&manifest(path)?)?)
            })?;
            app.run_with_peer(integers(args)?, captures, peer)?;
        }
        [command, path, export, args @ ..] if command == "call" => {
            let module = Module::compile_compact(&manifest(Path::new(path))?)?;
            let value = run_export(&module, &mut Workspace::default(), export, &integers(args)?)?;
            writeln!(io::stdout().lock(), "{value:?}")?;
        }
        [command, path, adapter, names, init, step, args @ ..] if command == "lines" => {
            let mut source = manifest(Path::new(path))?;
            append(Path::new(adapter), &mut source)?;
            let mut names_text = String::new();
            append(Path::new(names), &mut names_text)?;
            // Table indices are physical rows, including the final row without a newline.
            names_text.pop();
            let program = LineProgram {
                module: Module::compile_compact(&source)?,
                table: table(&names_text)?,
                init,
                step,
            };
            program.replay(&integers(args)?, io::stdin().lock(), io::stdout().lock())?;
        }
        [command, path, args @ ..] if command == "series" => {
            let module = Module::compile_compact(&manifest(Path::new(path))?)?;
            fluxr::structured_series::run(&module, args.iter().cloned(), io::stdout().lock())?;
        }
        _ => {
            #[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
            return Err(format!("{USAGE}{NATIVE_USAGE}").into());
            #[cfg(not(all(feature = "native-runtime", not(target_arch = "wasm32"))))]
            return Err(USAGE.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_replay_keeps_policy_in_the_module() -> Result<(), Box<dyn Error>> {
        let program = LineProgram {
            module: Module::compile_compact(
                "v2 (enum Init denied ready:i64) (rec Step next:i64 receipt:i64)
                 (pub start (n:i64) #Init () (ret (tag Init.ready n)))
                 (pub step (n:i64 code:i64) Step ()
                   (when (== code 0) (ret (new Step n -1)))
                   (ret (new Step (+ n 2) (+ n 2))))",
            )?,
            table: table("ignore\tmetadata\nadvance")?,
            init: "start",
            step: "step",
        };
        let mut output = Vec::new();
        let input = b"ignore\r\nadvance\nadvance\nunknown\n";
        let error = program
            .replay(&[Value::I64(3)], &input[..], &mut output)
            .unwrap_err();
        assert_eq!(
            String::from_utf8(output)?,
            "I64(3)\nI64(-1)\nI64(5)\nI64(7)\n"
        );
        assert_eq!(error.to_string(), "unknown order \"unknown\"");
        Ok(())
    }
}
