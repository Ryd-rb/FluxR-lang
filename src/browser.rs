use super::{BoundTrack, Program, RuleProgram, telemetry};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Scene(Program);

#[wasm_bindgen]
pub struct TimelineTrack(BoundTrack);

#[wasm_bindgen]
impl TimelineTrack {
    pub fn sample(&self, time_ms: u32) -> i32 {
        self.0.sample(time_ms)
    }
}

#[wasm_bindgen]
pub struct Rule(RuleProgram);

#[wasm_bindgen]
impl Rule {
    pub fn compile(source: &str, structured: bool) -> Result<Rule, JsError> {
        let program = if structured {
            RuleProgram::compile_structured(source)
        } else {
            RuleProgram::compile_text(source)
        }?;
        Ok(Rule(program))
    }

    pub fn decode(bytes: &[u8]) -> Result<Rule, JsError> {
        Ok(Rule(RuleProgram::decode(bytes)?))
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn replay(&self, assertions: &str) -> Result<u32, JsError> {
        Ok(self.0.replay(assertions)?)
    }
}

#[wasm_bindgen(js_name = configureTelemetry)]
pub fn configure_telemetry(selection: &str) -> Result<(), JsError> {
    telemetry::configure(selection).map_err(|error| JsError::new(&error))
}

#[wasm_bindgen(js_name = telemetrySnapshot)]
pub fn telemetry_snapshot() -> String {
    telemetry::snapshot().to_string()
}

#[wasm_bindgen(js_name = telemetryReloadEnabled)]
pub fn telemetry_reload_enabled() -> bool {
    telemetry::Feature::Reload.enabled()
}

#[wasm_bindgen(js_name = recordTelemetryReload)]
pub fn record_telemetry_reload(accepted: bool, elapsed_us: u32) {
    telemetry::reload(accepted, 0, u64::from(elapsed_us));
}

#[wasm_bindgen]
impl Scene {
    pub fn compile(source: &str, structured: bool) -> Result<Scene, JsError> {
        let program = if structured {
            Program::compile_structured(source)
        } else {
            Program::compile_text(source)
        }?;
        Ok(Scene(program))
    }

    pub fn decode(bytes: &[u8]) -> Result<Scene, JsError> {
        Ok(Scene(Program::decode(bytes)?))
    }

    pub fn text(&self) -> String {
        self.0.text()
    }

    pub fn structured(&self) -> String {
        self.0.structured()
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn sample(&self, object: u16, field: u16, time_ms: u32) -> Option<i32> {
        self.0.sample((object, field), time_ms)
    }

    pub fn bind_track(&self, object: u16, field: u16) -> Result<TimelineTrack, JsError> {
        self.0
            .bind_track((object, field))
            .map(TimelineTrack)
            .ok_or_else(|| JsError::new("Missing timeline track"))
    }

    pub fn replay(&self, assertions: &str) -> Result<u32, JsError> {
        Ok(self.0.replay(assertions)?)
    }
}
