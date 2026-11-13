//! Test-only public-API bridge for tools/semantic_campaign.py.
//!
//! This process reports observations, never expected semantics. It accepts one
//! bounded JSON request on stdin, writes one response to stdout, and owns no
//! files. The Python campaign owns deadlines, oracles, replay and reduction.
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{self, Read, Write};

const REQUEST_SCHEMA: &str = "fluxr-semantic-request-v1";
const RESPONSE_SCHEMA: &str = "fluxr-semantic-response-v1";
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROGRAMS: usize = 256;
const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_CALLS_PER_PROGRAM: usize = 512;
const MAX_TOTAL_CALLS: usize = 8192;
const MAX_LANES: usize = 16;
const MAX_INSTRUCTIONS: u64 = 1_000_000;
const MAX_CELLS: usize = 65_536;
const MAX_FRAMES: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scalar {
    I64(i64),
    Bool(bool),
}

impl Scalar {
    fn value(&self) -> Value {
        match self {
            Self::I64(value) => Value::I64(*value),
            Self::Bool(value) => Value::Bool(*value),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    source: String,
    export: String,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    arguments: Vec<Scalar>,
    inputs: Vec<Scalar>,
    limits: RequestLimits,
}

// Keep the wire contract local to this test adapter even if the runtime's
// serialized Limits type gains fields or defaults in a future release.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestLimits {
    instructions: u64,
    cells: usize,
    frames: usize,
}

impl RequestLimits {
    fn runtime(self) -> Limits {
        Limits {
            instructions: self.instructions,
            cells: self.cells,
            frames: self.frames,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum Outcome {
    Value { value: Scalar },
    Trap { trap: String },
}

#[derive(Debug, Serialize)]
struct CallOutput {
    portable: Outcome,
    prepared: Outcome,
    route: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum CaseOutput {
    Calls {
        id: String,
        calls: Vec<CallOutput>,
    },
    CompileError {
        id: String,
        compile_error: String,
    },
    PrepareError {
        id: String,
        prepare_error: String,
    },
}

#[derive(Debug, Serialize)]
struct Driver {
    typed_jit: bool,
    os: &'static str,
    arch: &'static str,
}

#[derive(Debug, Serialize)]
struct Response {
    schema: &'static str,
    driver: Driver,
    cases: Vec<CaseOutput>,
}

fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("semantic driver request exceeds 4 MiB".into());
    }
    let request: Request =
        serde_json::from_slice(bytes).map_err(|error| format!("semantic request: {error}"))?;
    if request.schema != REQUEST_SCHEMA {
        return Err("semantic driver request schema differs".into());
    }
    if request.cases.is_empty() || request.cases.len() > MAX_PROGRAMS {
        return Err("semantic driver requires 1..=256 programs".into());
    }
    let mut ids = BTreeSet::new();
    let mut total_calls = 0usize;
    for case in &request.cases {
        if case.id.is_empty() || case.id.len() > 128 || !ids.insert(&case.id) {
            return Err("semantic driver case ids must be unique and 1..=128 bytes".into());
        }
        if case.source.is_empty() || case.source.len() > MAX_SOURCE_BYTES {
            return Err("semantic driver source must be 1..=16384 bytes".into());
        }
        if case.export.is_empty()
            || case.export.len() > 64
            || !case
                .export
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("semantic driver export must be a bounded identifier".into());
        }
        if case.calls.is_empty() || case.calls.len() > MAX_CALLS_PER_PROGRAM {
            return Err("semantic driver requires 1..=512 calls per program".into());
        }
        total_calls += case.calls.len();
        if total_calls > MAX_TOTAL_CALLS {
            return Err("semantic driver total calls exceed 8192".into());
        }
        for call in &case.calls {
            if call.arguments.len() > MAX_LANES || call.inputs.len() > MAX_LANES {
                return Err("semantic driver boundary exceeds 16 scalar lanes per slice".into());
            }
            if call.limits.instructions > MAX_INSTRUCTIONS
                || call.limits.cells > MAX_CELLS
                || call.limits.frames > MAX_FRAMES
            {
                return Err("semantic driver invocation limits exceed campaign caps".into());
            }
        }
    }
    Ok(request)
}

fn outcome(result: Result<Value, fluxr::Error>) -> Result<Outcome, String> {
    match result {
        Ok(Value::I64(value)) => Ok(Outcome::Value {
            value: Scalar::I64(value),
        }),
        Ok(Value::Bool(value)) => Ok(Outcome::Value {
            value: Scalar::Bool(value),
        }),
        Ok(_) => Err("semantic driver received a non-scalar result".into()),
        Err(error) => Ok(Outcome::Trap {
            trap: error.to_string(),
        }),
    }
}

fn execute(request: Request) -> Result<Response, String> {
    let mut cases = Vec::with_capacity(request.cases.len());
    for case in request.cases {
        let module = match Module::compile_compact(&case.source) {
            Ok(module) => module,
            Err(error) => {
                cases.push(CaseOutput::CompileError {
                    id: case.id,
                    compile_error: error.to_string(),
                });
                continue;
            }
        };
        let prepared = match module.prepare_call(&case.export) {
            Ok(prepared) => prepared,
            Err(error) => {
                cases.push(CaseOutput::PrepareError {
                    id: case.id,
                    prepare_error: error.to_string(),
                });
                continue;
            }
        };
        // Both paths keep their own workspace across success and failure. No
        // assertion or expected value is supplied by the implementation here.
        let mut portable_workspace = Workspace::default();
        let mut prepared_workspace = Workspace::default();
        let mut calls = Vec::with_capacity(case.calls.len());
        for call in case.calls {
            let arguments: Vec<_> = call.arguments.iter().map(Scalar::value).collect();
            let inputs: Vec<_> = call.inputs.iter().map(Scalar::value).collect();
            let limits = call.limits.runtime();
            let portable = module.run_portable_with(
                Invocation {
                    export: &case.export,
                    inputs: &inputs,
                    arguments: &arguments,
                    limits,
                },
                &mut portable_workspace,
            );
            let (actual, route) = prepared.run_with_route(
                &inputs,
                &arguments,
                limits,
                &mut prepared_workspace,
            );
            calls.push(CallOutput {
                portable: outcome(portable)?,
                prepared: outcome(actual)?,
                route: format!("{route:?}"),
            });
        }
        cases.push(CaseOutput::Calls { id: case.id, calls });
    }
    Ok(Response {
        schema: RESPONSE_SCHEMA,
        driver: Driver {
            typed_jit: cfg!(feature = "typed-jit"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        cases,
    })
}

fn serve(input: &mut impl Read, output: &mut impl Write) -> Result<(), String> {
    let mut bytes = Vec::new();
    input
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("semantic driver input: {error}"))?;
    let response = execute(parse_request(&bytes)?)?;
    let bytes =
        serde_json::to_vec(&response).map_err(|error| format!("semantic driver output: {error}"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("semantic driver response exceeds 8 MiB".into());
    }
    output
        .write_all(&bytes)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|error| format!("semantic driver output: {error}"))
}

fn main() {
    let result = if std::env::args_os().len() != 1 {
        Err("usage: semantic_campaign_driver < request.json > response.json".into())
    } else {
        serve(&mut io::stdin().lock(), &mut io::stdout().lock())
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    fn request(source: &str, calls: Vec<Json>) -> Json {
        json!({
            "schema": REQUEST_SCHEMA,
            "cases": [{"id":"fixture", "source":source, "export":"calc", "calls":calls}]
        })
    }

    fn call(arguments: Json, instructions: u64, cells: usize, frames: usize) -> Json {
        json!({"arguments":arguments,"inputs":[],"limits":{
            "instructions":instructions,"cells":cells,"frames":frames
        }})
    }

    fn observe(request: Json) -> Json {
        let input = serde_json::to_vec(&request).unwrap();
        let mut output = Vec::new();
        serve(&mut input.as_slice(), &mut output).unwrap();
        serde_json::from_slice(&output).unwrap()
    }

    #[test]
    fn reports_boolean_results_and_actual_routes() {
        let response = observe(request(
            "v2 (pub calc (flag:bool x:i64) bool = (& flag (< x 3)))",
            vec![
                call(json!([{"bool":true},{"i64":2}]), 20, 20, 1),
                call(json!([{"bool":false},{"i64":2}]), 20, 20, 1),
            ],
        ));
        assert_eq!(response["schema"], RESPONSE_SCHEMA);
        assert_eq!(response["driver"]["typed_jit"], cfg!(feature = "typed-jit"));
        for (index, expected) in [true, false].into_iter().enumerate() {
            let row = &response["cases"][0]["calls"][index];
            assert_eq!(row["portable"], json!({"value":{"bool":expected}}));
            assert_eq!(row["prepared"], row["portable"]);
            assert!(matches!(
                row["route"].as_str(),
                Some("Portable" | "TypedJit" | "NativeScalar")
            ));
        }
    }

    #[test]
    fn preserves_success_trap_success_workspace_reuse() {
        let response = observe(request(
            "v2 (pub calc (x:i64 y:i64) i64 = (/ x y))",
            vec![
                call(json!([{"i64":21},{"i64":3}]), 4, 6, 1),
                call(json!([{"i64":21},{"i64":0}]), 4, 6, 1),
                call(json!([{"i64":-21},{"i64":3}]), 4, 6, 1),
            ],
        ));
        let rows = response["cases"][0]["calls"].as_array().unwrap();
        let expected = [
            json!({"value":{"i64":7}}),
            json!({"trap":"invalid or overflowing integer arithmetic"}),
            json!({"value":{"i64":-7}}),
        ];
        for (row, expected) in rows.iter().zip(expected) {
            assert_eq!(row["portable"], expected);
            assert_eq!(row["prepared"], expected);
        }
        assert_eq!(rows[1]["route"], "Portable");
    }

    #[test]
    fn retains_exact_first_trap_admission_and_arithmetic_boundaries() {
        // Hand-frozen boundaries: one argument cell, two frame cells, four
        // fuel, one result cell. Arithmetic can fail after fuel three.
        let response = observe(request(
            "v2 (pub calc (x:i64) i64 = (+ x 1))",
            vec![
                call(json!([{"bool":true}]), 0, 0, 0),
                call(json!([{"bool":true}]), 0, 1, 0),
                call(json!([{"i64":0}]), 0, 1, 0),
                call(json!([{"i64":i64::MAX}]), 2, 3, 1),
                call(json!([{"i64":i64::MAX}]), 3, 3, 1),
                call(json!([{"i64":0}]), 4, 3, 1),
                call(json!([{"i64":0}]), 4, 4, 1),
            ],
        ));
        let expected = [
            json!({"trap":"module cell budget exhausted"}),
            json!({"trap":"host value differs from I64"}),
            json!({"trap":"module frame limit exceeded"}),
            json!({"trap":"module instruction budget exhausted"}),
            json!({"trap":"invalid or overflowing integer arithmetic"}),
            json!({"trap":"module cell budget exhausted"}),
            json!({"value":{"i64":1}}),
        ];
        for (row, expected) in response["cases"][0]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .zip(expected)
        {
            assert_eq!(row["portable"], expected);
            assert_eq!(row["prepared"], expected);
        }
    }

    #[test]
    fn compile_and_preparation_errors_are_separate_from_execution_traps() {
        let row = call(json!([]), 10, 10, 1);
        let mut value = request("v2 (pub calc () i64 = 1)", vec![row.clone()]);
        value["cases"][0]["export"] = json!("missing");
        let response = observe(value);
        assert_eq!(
            response["cases"][0]["prepare_error"],
            "unknown export missing"
        );
        assert!(response["cases"][0].get("calls").is_none());
        let response = observe(request("v2 invalid", vec![row]));
        assert!(response["cases"][0]["compile_error"].is_string());
        assert!(response["cases"][0].get("calls").is_none());
    }

    #[test]
    fn rejects_unknown_fields_duplicate_fields_and_non_scalar_values() {
        let valid = request(
            "v2 (pub calc (x:i64) i64 = x)",
            vec![call(json!([{"i64":1}]), 10, 10, 1)],
        );
        let mut unknown = valid.clone();
        unknown["extra"] = json!(true);
        assert!(parse_request(&serde_json::to_vec(&unknown).unwrap()).is_err());
        let mut unknown_limit = valid.clone();
        unknown_limit["cases"][0]["calls"][0]["limits"]["extra"] = json!(true);
        assert!(parse_request(&serde_json::to_vec(&unknown_limit).unwrap()).is_err());
        let mut bad_value = valid.clone();
        bad_value["cases"][0]["calls"][0]["arguments"] = json!([{"u64":1}]);
        assert!(parse_request(&serde_json::to_vec(&bad_value).unwrap()).is_err());
        bad_value["cases"][0]["calls"][0]["arguments"] = json!([{"i64":true}]);
        assert!(parse_request(&serde_json::to_vec(&bad_value).unwrap()).is_err());
        let bytes = serde_json::to_string(&valid).unwrap();
        let duplicate = bytes.replacen(
            "\"schema\":",
            "\"schema\":\"fluxr-semantic-request-v1\",\"schema\":",
            1,
        );
        assert!(parse_request(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn rejects_request_program_call_source_and_budget_overruns() {
        assert!(parse_request(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
        let valid = request(
            "v2 (pub calc (x:i64) i64 = x)",
            vec![call(json!([{"i64":1}]), 10, 10, 1)],
        );
        let mut wrong_schema = valid.clone();
        wrong_schema["schema"] = json!("fluxr-semantic-request-v999");
        assert!(parse_request(&serde_json::to_vec(&wrong_schema).unwrap()).is_err());
        let mut empty = valid.clone();
        empty["cases"] = json!([]);
        assert!(parse_request(&serde_json::to_vec(&empty).unwrap()).is_err());
        let mut too_many = valid.clone();
        too_many["cases"] = Json::Array(vec![valid["cases"][0].clone(); MAX_PROGRAMS + 1]);
        assert!(parse_request(&serde_json::to_vec(&too_many).unwrap()).is_err());
        let mut repeated = valid.clone();
        repeated["cases"] = Json::Array(vec![valid["cases"][0].clone(); 2]);
        assert!(parse_request(&serde_json::to_vec(&repeated).unwrap()).is_err());
        let mut large_source = valid.clone();
        large_source["cases"][0]["source"] = json!(" ".repeat(MAX_SOURCE_BYTES + 1));
        assert!(parse_request(&serde_json::to_vec(&large_source).unwrap()).is_err());
        let mut large_calls = valid.clone();
        large_calls["cases"][0]["calls"] = Json::Array(vec![
            valid["cases"][0]["calls"][0].clone();
            MAX_CALLS_PER_PROGRAM + 1
        ]);
        assert!(parse_request(&serde_json::to_vec(&large_calls).unwrap()).is_err());
        for (key, value) in [
            ("instructions", MAX_INSTRUCTIONS + 1),
            ("cells", MAX_CELLS as u64 + 1),
            ("frames", MAX_FRAMES as u64 + 1),
        ] {
            let mut overrun = valid.clone();
            overrun["cases"][0]["calls"][0]["limits"][key] = json!(value);
            assert!(parse_request(&serde_json::to_vec(&overrun).unwrap()).is_err());
        }
    }
}
