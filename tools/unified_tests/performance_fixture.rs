//! Identical native kernel text for direct-Rust and mandatory-Runtime arms.
//! Signed boundaries exercise the implemented AOT subset. Every measured input
//! is in 0..997: x*x+17 is at most 992_033, and the driver's largest permitted
//! checksum is at most 99_600_000_000. Neither intermediate nor sum can overflow.
use std::hint::black_box;
use std::time::Instant;

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn fluxr_unified_probe_kernel(x: i64) -> i64 {
    (x * x + 17) % 997
}

fn kernel(iterations: u64) {
    let started = Instant::now();
    let mut checksum = 0i64;
    for i in 0..iterations {
        checksum += black_box(fluxr_unified_probe_kernel(black_box((i % 997) as i64)));
    }
    println!("{{\"kind\":\"native_kernel\",\"iterations\":{iterations},\"elapsed_ns\":{},\"checksum\":{checksum}}}", started.elapsed().as_nanos());
}

#[cfg(feature = "unified")]
fn runtime_paths(iterations: u64) {
    use fluxr::runtime::{global, ExecutionRoute, Limits, Type, Value};
    use fluxr::structured::Workspace;
    let runtime = global();
    let started = Instant::now();
    runtime.register_host("arithmetic", vec![Type::I64], Type::I64, |arguments| {
        let Value::I64(x) = arguments[0] else { panic!("host argument type changed") };
        Ok(Value::I64(fluxr_unified_probe_kernel(x)))
    }).unwrap();
    runtime.install("injected", None, "fluxr 1\npub calc 1 x int int ret rem add mul x x 17 997\npub host_calc 1 x int int ret host arithmetic int 1 int x\n").unwrap();
    let bundled = runtime.snapshot("main.fluxr").unwrap();
    let injected = runtime.snapshot("injected").unwrap();
    let prepared_aot = bundled.prepare_export("calc").unwrap();
    let prepared_vm = injected.prepare_export("calc").unwrap();
    let prepared_host = injected.prepare_export("host_calc").unwrap();
    let prepared_ns = started.elapsed().as_nanos();
    println!("{{\"kind\":\"preparation\",\"elapsed_ns\":{prepared_ns}}}");
    assert!(bundled.info().unwrap().aot_exports.iter().any(|x| x == "calc"), "bundled checked kernel lacks an actual AOT binding");
    for path in ["prepared_aot", "prepared_vm", "prepared_host", "bundled_aot_snapshot", "injected_vm_snapshot", "injected_vm_registry", "injected_host_snapshot"] {
        let mut workspace = Workspace::default();
        let mut arguments = [Value::I64(0)];
        let mut checksum = 0i64;
        let mut observed_route = "";
        let expected_route = if path == "bundled_aot_snapshot" || path == "prepared_aot" { "aot" } else { "vm" };
        let start = Instant::now();
        for i in 0..iterations {
            arguments[0] = Value::I64(black_box((i % 997) as i64));
            let (value, revision, route) = if matches!(path, "prepared_aot" | "prepared_vm" | "prepared_host") {
                let handle = match path { "prepared_aot" => &prepared_aot, "prepared_vm" => &prepared_vm, _ => &prepared_host };
                let result = handle.call_with(&[], &arguments, Limits::default(), &mut workspace).unwrap();
                let route = match result.route {
                    ExecutionRoute::AheadOfTime => "aot", ExecutionRoute::Portable => "vm",
                    _ => panic!("unexpected prepared route"),
                };
                (result.value, result.revision, route)
            } else {
                let result = match path {
                    "bundled_aot_snapshot" => bundled.call_with("calc", &[], &arguments, Limits::default(), &mut workspace),
                    "injected_vm_snapshot" => injected.call_with("calc", &[], &arguments, Limits::default(), &mut workspace),
                    "injected_vm_registry" => runtime.call("injected", "calc", &[], &arguments, Limits::default()),
                    "injected_host_snapshot" => injected.call_with("host_calc", &[], &arguments, Limits::default(), &mut workspace),
                    _ => unreachable!(),
                }.unwrap();
                let route = match result.route.as_str() { "aot" => "aot", "vm" => "vm", _ => panic!("unexpected runtime route") };
                (result.value, result.revision, route)
            };
            assert_eq!(revision, 1);
            assert_eq!(route, expected_route, "runtime route changed inside a measured batch");
            let Value::I64(value) = value else { panic!("unexpected result type") };
            checksum += black_box(value);
            if i == 0 { observed_route = route; }
        }
        let elapsed_ns = start.elapsed().as_nanos();
        assert_eq!(observed_route, expected_route);
        println!("{{\"kind\":\"{path}\",\"route\":\"{observed_route}\",\"iterations\":{iterations},\"elapsed_ns\":{elapsed_ns},\"checksum\":{checksum}}}");
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("startup") => println!("ready"),
        Some("kernel") => kernel(arguments.next().unwrap().parse().unwrap()),
        #[cfg(feature = "unified")]
        Some("runtime") => runtime_paths(arguments.next().unwrap().parse().unwrap()),
        _ => panic!("expected startup, kernel ITERATIONS, or runtime ITERATIONS"),
    }
}
