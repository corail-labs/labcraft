use super::*;
use crate::value::ValueWriter;

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8(bytes.to_vec()).unwrap().lines().map(str::to_owned).collect()
}

const HEADER: Header<'static> =
    Header { run: "r2", function: "check_voltage", mode: "deterministic", files: &[("check_voltage.lcf", "ab"), ("Verdict.lct", "cd")] };

/// The lines of the example of commands.md 8, written by hand in the order of the writer: `seq`, `t_ms`, `event`,
/// `frame`, then what the event holds.
#[test]
fn the_example_of_the_catalog_writes_as_it_reads() {
    let mut writer = TraceWriter::new(Vec::new(), &HEADER);
    let resource = |w: &mut ValueWriter<'_>| w.string("SIM::DMM0");
    let low = |w: &mut ValueWriter<'_>| w.f64(220.0, Some("V"));
    let none = |w: &mut ValueWriter<'_>| w.null();
    let session = |w: &mut ValueWriter<'_>| {
        w.begin_object();
        w.key("opaque");
        w.string("visa.session");
        w.key("resource");
        w.string("SIM::DMM0");
        w.key("open");
        w.bool(true);
        w.end();
    };
    let raw = |w: &mut ValueWriter<'_>| w.f64(230.071217, None);
    let inputs = [
        Input { name: "resource", ty: "string", value: &resource },
        Input { name: "low", ty: "f64[V]", value: &low },
        Input { name: "err", ty: "error", value: &none },
    ];
    let types = [("dmm", "visa.session"), ("e1", "error"), ("raw", "f64")];
    writer.event(0, &Event::Call { frame: 0, caller: None, function: "check_voltage", inputs: &inputs, types: &types });
    let caller = Caller { frame: 0, node: "dmm", iter: &[] };
    writer.event(0, &Event::Call { frame: 1, caller: Some(caller), function: "visa.open", inputs: &inputs[..1], types: &[] });
    writer.event(0, &Event::Ran { frame: 0, node: "dmm", iter: &[], out: &[("dmm", &session), ("e1", &none)], types: &[] });
    writer.event(1_500_000, &Event::Ran { frame: 0, node: "raw", iter: &[2, 0], out: &[("raw", &raw)], types: &[("raw", "f64")] });
    writer.event(1_500_000, &Event::Arm { frame: 0, node: "measured", iter: &[], arm: "true" });
    writer.event(2_000_000, &Event::Set { frame: 0, element: "stop", value: &|w: &mut ValueWriter<'_>| w.bool(true), by: "set" });
    writer.event(2_000_000, &Event::Skipped { frame: 0, node: "#close", iter: &[], out: &[("e2", &none)], types: &[] });
    writer.end(2_000_250, "ok");
    assert_eq!(writer.events(), 8);
    assert!(!writer.truncated());
    let expected = [
        r#"{"schema":"labcraft-trace/1","run":"r2","function":"check_voltage","mode":"deterministic","files":[{"file":"check_voltage.lcf","sha256":"ab"},{"file":"Verdict.lct","sha256":"cd"}]}"#,
        r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"check_voltage","in":[{"name":"resource","type":"string","value":"SIM::DMM0"},{"name":"low","type":"f64[V]","value":"220.0 V"},{"name":"err","type":"error","value":null}],"types":{"dmm":"visa.session","e1":"error","raw":"f64"}}"#,
        r#"{"seq":2,"t_ms":0.0,"event":"call","frame":1,"parent":0,"node":"dmm","function":"visa.open","in":[{"name":"resource","type":"string","value":"SIM::DMM0"}],"types":{}}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"dmm":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e1":null}}"#,
        r#"{"seq":4,"t_ms":1.5,"event":"ran","frame":0,"node":"raw","iter":[2,0],"out":{"raw":230.071217},"types":{"raw":"f64"}}"#,
        r#"{"seq":5,"t_ms":1.5,"event":"arm","frame":0,"node":"measured","arm":"true"}"#,
        r#"{"seq":6,"t_ms":2.0,"event":"set","frame":0,"element":"stop","value":true,"by":"set"}"#,
        r##"{"seq":7,"t_ms":2.0,"event":"skipped","frame":0,"node":"#close","out":{"e2":null}}"##,
        r#"{"seq":8,"t_ms":2.00025,"event":"end","status":"ok"}"#,
    ];
    assert_eq!(lines(&writer.into_inner()), expected);
}

fn big(w: &mut ValueWriter<'_>) {
    w.string(&"x".repeat(900));
}

/// A line of 1,100 bytes, 64 of them kept for its end. Before the first value, 66 bytes: the value of 902 and the reserve
/// of 14 fit in the 970 left. Before the second, 973: 63 left, a cut. Then `,"types":{` ends at 997, and an entry of 56
/// bytes passes the 39 left: dropped. Worked out with Python from the text of the line.
#[test]
fn a_line_past_its_bound_cuts_its_values_then_drops_its_entries() {
    let limits = WriteLimits { line: 1100, ..WriteLimits::default() };
    let mut writer = TraceWriter::with_limits(Vec::new(), &HEADER, limits);
    let ty = "t".repeat(50);
    let types = [("a", ty.as_str()), ("b", ty.as_str())];
    writer.event(0, &Event::Ran { frame: 0, node: "a", iter: &[], out: &[("a", &big), ("b", &big)], types: &types });
    let written = lines(&writer.into_inner());
    let line = &written[1];
    assert_eq!(line.len(), 1016);
    let json: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(json["out"]["a"].as_str().unwrap().len(), 900);
    assert_eq!(json["out"]["b"], serde_json::json!({ "$cut": true }));
    assert_eq!(json["types"], serde_json::json!({}));
    assert_eq!(json["truncated"], true);
}

/// Names that cannot fit even with their values cut: the line keeps only what places it.
#[test]
fn a_line_that_cannot_fit_keeps_only_what_places_it() {
    let limits = WriteLimits { line: 300, ..WriteLimits::default() };
    let mut writer = TraceWriter::with_limits(Vec::new(), &HEADER, limits);
    let long = "n".repeat(100);
    let none = |w: &mut ValueWriter<'_>| w.null();
    let out: Vec<(&str, &dyn WriteValue)> = (0..5).map(|_| (long.as_str(), &none as &dyn WriteValue)).collect();
    writer.event(0, &Event::Ran { frame: 3, node: "y", iter: &[1], out: &out, types: &[] });
    let written = lines(&writer.into_inner());
    assert_eq!(written[1], r#"{"seq":1,"t_ms":0.0,"event":"ran","frame":3,"node":"y","truncated":true}"#);
}

#[test]
fn the_files_of_the_first_line_are_dropped_past_its_bound() {
    let files: Vec<(&str, &str)> = vec![("a.lcf", "0123456789"); 20];
    let header = Header { files: &files, ..HEADER };
    let writer = TraceWriter::with_limits(Vec::new(), &header, WriteLimits { line: 300, ..WriteLimits::default() });
    let written = lines(&writer.into_inner());
    let json: serde_json::Value = serde_json::from_str(&written[0]).unwrap();
    assert!(written[0].len() <= 300);
    // What comes before the files takes 99 bytes, each entry 38 and a comma, the tail 64: 99 + 38 + 39 + 39 = 215 fits in 236.
    assert_eq!(json["files"].as_array().unwrap().len(), 3);
    assert_eq!(json["truncated"], true);
}

#[test]
fn past_its_events_a_trace_says_truncated_then_only_its_end() {
    let limits = WriteLimits { events: 2, ..WriteLimits::default() };
    let mut writer = TraceWriter::with_limits(Vec::new(), &HEADER, limits);
    for _ in 0..5 {
        writer.event(0, &Event::Arm { frame: 0, node: "m", iter: &[], arm: "_" });
    }
    writer.end(0, "ok");
    writer.end(0, "ok");
    assert!(writer.truncated());
    assert_eq!(writer.events(), 4);
    let written = lines(&writer.into_inner());
    assert_eq!(written.len(), 5);
    assert_eq!(written[3], r#"{"seq":3,"t_ms":0.0,"event":"truncated"}"#);
    assert_eq!(written[4], r#"{"seq":4,"t_ms":0.0,"event":"end","status":"ok"}"#);
}

#[test]
fn past_its_bytes_a_trace_says_truncated() {
    let limits = WriteLimits { file: 2400, ..WriteLimits::default() };
    let mut writer = TraceWriter::with_limits(Vec::new(), &HEADER, limits);
    for _ in 0..10 {
        writer.event(0, &Event::Ran { frame: 0, node: "a", iter: &[], out: &[("a", &big)], types: &[] });
    }
    writer.end(0, "ok");
    let bytes = writer.into_inner();
    assert!(bytes.len() <= 2400, "{}", bytes.len());
    let written = lines(&bytes);
    // The first line takes 181 bytes with its end, each event 971; an event is written when 256 bytes stay for
    // `truncated` and `end`: 181 + 2 × 971 + 256 = 2,379 fits in 2,400, a third does not (3,350).
    assert_eq!(written.len(), 5, "{written:?}");
    assert!(written[3].contains("\"truncated\""));
}

struct Broken;

impl io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("disk full"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_sink_that_fails_stops_the_trace_and_nothing_else() {
    let mut writer = TraceWriter::new(Broken, &HEADER);
    writer.event(0, &Event::Arm { frame: 0, node: "m", iter: &[], arm: "_" });
    writer.end(0, "ok");
    assert_eq!(writer.error().map(ToString::to_string).as_deref(), Some("disk full"));
}

/// The first line names the trace whole, whatever its length, and stays JSON.
#[test]
fn the_first_line_stays_json_past_its_bound() {
    let run = "r".repeat(400);
    let header = Header { run: &run, ..HEADER };
    let writer = TraceWriter::with_limits(Vec::new(), &header, WriteLimits { line: 300, ..WriteLimits::default() });
    let written = lines(&writer.into_inner());
    let json: serde_json::Value = serde_json::from_str(&written[0]).unwrap();
    assert_eq!(
        (json["run"].as_str().map(str::len), json["files"].as_array().map(Vec::len), &json["truncated"]),
        (Some(400), Some(0), &serde_json::json!(true))
    );
}

struct Unflushed(bool);

impl io::Write for Unflushed {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0 = true;
        Err(io::Error::other("cannot flush"))
    }
}

#[test]
fn the_end_flushes_the_sink_and_says_when_it_fails() {
    let mut writer = TraceWriter::new(Unflushed(false), &HEADER);
    writer.end(0, "ok");
    assert_eq!(writer.error().map(ToString::to_string).as_deref(), Some("cannot flush"));
    assert!(writer.into_inner().0);
}
