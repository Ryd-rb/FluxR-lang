use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use fluxr::{Program, RuleProgram};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::File,
    io::{Read, Write},
};

const MAX_REPLAY_BYTES: usize = 256 * 1024;
const MAX_CASES: usize = 256;

pub(crate) struct Options<'a> {
    pub receipt: bool,
    pub replay: Option<&'a str>,
}

pub(crate) enum Legacy {
    Timeline(Program),
    Rule(RuleProgram),
    Structured(Module),
}

#[derive(Serialize)]
struct Receipt<'a> {
    version: u8,
    artifact: &'a str,
    input_sha256: String,
    source_sha256: String,
    canonical_sha256: String,
    replay: Option<ReplayReceipt>,
}

#[derive(Serialize)]
struct HirReceipt<'a> {
    version: u8,
    ir: &'a str,
    artifact: &'a str,
    input_sha256: String,
    ir_sha256: String,
    canonical_sha256: String,
    replay: Option<ReplayReceipt>,
}

#[derive(Serialize)]
struct ReplayReceipt {
    sha256: String,
    cases: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    export: String,
    inputs: Vec<Value>,
    arguments: Vec<Value>,
    #[serde(default)]
    limits: Limits,
}

impl Legacy {
    pub(crate) fn write_hir(
        self,
        input: &[u8],
        output: &str,
        options: Options<'_>,
    ) -> Result<(), Box<dyn Error>> {
        let (artifact, hir) = match &self {
            Self::Rule(rule) => ("FXD", Hir::from_rule(rule.clone())?),
            Self::Structured(module) => ("FSM", Hir::from_structured(module.clone())),
            Self::Timeline(_) => return Err("FXH does not support timelines".into()),
        };
        let wire = hir.encode()?;
        let restored = match Hir::decode(&wire)?.into_artifact() {
            Artifact::Rule(rule) => Self::Rule(rule),
            Artifact::Structured(module) => Self::Structured(module),
        };
        let canonical = self.encode();
        let replay = self.verify(&restored, &canonical, options.replay)?;
        let receipt = options
            .receipt
            .then(|| {
                serde_json::to_string(&HirReceipt {
                    version: 1,
                    ir: "FXH",
                    artifact,
                    input_sha256: digest(input),
                    ir_sha256: digest(&wire),
                    canonical_sha256: digest(&canonical),
                    replay,
                })
            })
            .transpose()?;
        publish(output, &wire, receipt)
    }

    pub(crate) fn write(
        self,
        input: &[u8],
        output: &str,
        options: Options<'_>,
    ) -> Result<(), Box<dyn Error>> {
        let (artifact, source, canonical, restored) = match &self {
            Self::Timeline(program) => {
                let source = program.compact_text();
                let restored = Program::compile_text(&source).map(Self::Timeline);
                ("LSP", source, program.encode(), restored)
            }
            Self::Rule(rule) => {
                let source = rule.text();
                let restored = RuleProgram::compile_text(&source).map(Self::Rule);
                ("FXD", source, rule.encode(), restored)
            }
            Self::Structured(module) => {
                let source = module
                    .compact_text()
                    .map_err(|error| format!("migration refused: {error}"))?;
                let restored = Module::compile_compact(&source).map(Self::Structured);
                ("FSM", source, module.encode(), restored)
            }
        };
        let restored = restored.map_err(|error| format!("migration refused: {error}"))?;
        let replay = self.verify(&restored, &canonical, options.replay)?;
        let receipt = options
            .receipt
            .then(|| {
                serde_json::to_string(&Receipt {
                    version: 1,
                    artifact,
                    input_sha256: digest(input),
                    source_sha256: digest(source.as_bytes()),
                    canonical_sha256: digest(&canonical),
                    replay,
                })
            })
            .transpose()?;
        publish(output, source.as_bytes(), receipt)
    }

    fn verify(
        &self,
        restored: &Self,
        canonical: &[u8],
        replay: Option<&str>,
    ) -> Result<Option<ReplayReceipt>, Box<dyn Error>> {
        if restored.encode() != canonical {
            return Err("migration refused: canonical artifact differs".into());
        }
        replay
            .map(|path| self.replay(restored, path))
            .transpose()
            .map_err(|error| format!("migration refused: {error}").into())
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Timeline(program) => program.encode(),
            Self::Rule(rule) => rule.encode(),
            Self::Structured(module) => module.encode(),
        }
    }

    fn replay(&self, restored: &Self, path: &str) -> Result<ReplayReceipt, Box<dyn Error>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take((MAX_REPLAY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_REPLAY_BYTES {
            return Err("replay exceeds byte limit".into());
        }
        let cases = match (self, restored) {
            (Self::Timeline(old), Self::Timeline(new)) => {
                compare::<(u16, u16, u32), _>(&bytes, |(object, field, time)| {
                    Ok(old.sample((object, field), time) == new.sample((object, field), time))
                })?
            }
            (Self::Rule(old), Self::Rule(new)) => compare::<Vec<i64>, _>(&bytes, |inputs| {
                Ok(old.evaluate(&inputs) == new.evaluate(&inputs))
            })?,
            (Self::Structured(old), Self::Structured(new)) => {
                let mut old_workspace = Workspace::default();
                let mut new_workspace = Workspace::default();
                let ceiling = Limits::default();
                compare::<Call, _>(&bytes, |call| {
                    if call.limits.instructions > ceiling.instructions
                        || call.limits.cells > ceiling.cells
                        || call.limits.frames > ceiling.frames
                    {
                        return Err("structured replay exceeds default limits".into());
                    }
                    let invocation = || Invocation {
                        export: &call.export,
                        inputs: &call.inputs,
                        arguments: &call.arguments,
                        limits: call.limits,
                    };
                    Ok(old.run_portable_with(invocation(), &mut old_workspace)
                        == new.run_portable_with(invocation(), &mut new_workspace))
                })?
            }
            _ => return Err("migration replay changed artifact kind".into()),
        };
        Ok(ReplayReceipt {
            sha256: digest(&bytes),
            cases,
        })
    }
}

pub(crate) fn restore_hir(bytes: &[u8], output: &str) -> Result<(), Box<dyn Error>> {
    let artifact = match Hir::decode(bytes)?.into_artifact() {
        Artifact::Rule(rule) => rule.encode(),
        Artifact::Structured(module) => module.encode(),
    };
    publish(output, &artifact, None)
}

fn publish(output: &str, bytes: &[u8], receipt: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut file = File::create_new(output)?;
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let _ = std::fs::remove_file(output);
        return Err(error.into());
    }
    if let Some(receipt) = receipt {
        println!("{receipt}");
    }
    Ok(())
}

fn compare<T: DeserializeOwned, F>(bytes: &[u8], mut matches: F) -> Result<usize, Box<dyn Error>>
where
    F: FnMut(T) -> Result<bool, Box<dyn Error>>,
{
    let cases: Vec<T> = serde_json::from_slice(bytes)?;
    let count = cases.len();
    if !(1..=MAX_CASES).contains(&count) {
        return Err("replay case count outside limits".into());
    }
    for (index, case) in cases.into_iter().enumerate() {
        if !matches(case)? {
            return Err(format!("replay case {} differs", index + 1).into());
        }
    }
    Ok(count)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
