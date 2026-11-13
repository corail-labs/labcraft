use super::*;
use crate::value::ValueWriter;
use crate::write::{Event, Header, Input, TraceWriter};

fn wire(name: &str, ty: &str) -> ExpectedWire {
    ExpectedWire { name: name.into(), ty: ty.into() }
}

fn node(name: &str, loops: &[u32], arms: &[(u32, &str)], wires: &[(&str, &str)]) -> ExpectedNode {
    ExpectedNode {
        name: name.into(),
        arms: arms.iter().map(|(node, arm)| ArmOf { node: *node, arm: (*arm).into() }).collect(),
        loops: loops.to_vec(),
        wires: wires.iter().map(|(name, ty)| wire(name, ty)).collect(),
    }
}

const DMM: u32 = 0;
const READINGS: u32 = 1;
const RAW: u32 = 2;
const V: u32 = 3;
const X_TRUE: u32 = 4;
const X_FALSE: u32 = 5;

/// `dmm` opens a session; a loop `readings` reads `raw` at each iteration, and a `match` `v` binds `x` in each arm, with
/// another type in each.
fn expected() -> Expected {
    Expected {
        function: "f".into(),
        inputs: vec![wire("resource", "string"), wire("low", "f64[V]")],
        nodes: vec![
            node("dmm", &[], &[], &[("dmm", "visa.session"), ("e1", "error")]),
            node("readings", &[], &[], &[("readings", "[f64]")]),
            node("raw", &[READINGS], &[], &[("raw", "f64")]),
            node("v", &[READINGS], &[], &[("v", "string")]),
            node("x", &[READINGS], &[(V, "true")], &[("x", "string")]),
            node("x", &[READINGS], &[(V, "false")], &[("x", "i32")]),
        ],
    }
}

const FIRST: &str = r#"{"schema":"labcraft-trace/1","run":"r1","function":"f","mode":"deterministic","files":[{"file":"f.lcf","sha256":"ab"}]}"#;
const CALL: &str = r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"f","in":[{"name":"resource","type":"string","value":"SIM::DMM0"},{"name":"low","type":"f64[V]","value":"220.0 V"}],"types":{"dmm":"visa.session","e1":"error","readings":"[f64]","raw":"f64","v":"string"}}"#;

fn read(lines: &[&str]) -> Trace {
    read_with(lines, ReadLimits::default())
}

fn read_with(lines: &[&str], limits: ReadLimits) -> Trace {
    let mut reader = Reader::new(expected(), limits);
    reader.feed(lines.join("\n").as_bytes());
    reader.feed(b"\n");
    reader.finish()
}

/// What the writer writes, the reader reads: the two halves of the crate cannot drift apart.
#[test]
fn what_the_writer_writes_the_reader_reads() {
    let header = Header { run: "r1", function: "f", mode: "deterministic", files: &[("f.lcf", "ab")] };
    let mut writer = TraceWriter::new(Vec::new(), &header);
    let resource = |w: &mut ValueWriter<'_>| w.string("SIM::DMM0");
    let low = |w: &mut ValueWriter<'_>| w.f64(220.0, Some("V"));
    let inputs = [Input { name: "resource", ty: "string", value: &resource }, Input { name: "low", ty: "f64[V]", value: &low }];
    let types = [("dmm", "visa.session"), ("e1", "error"), ("readings", "[f64]"), ("raw", "f64"), ("v", "string")];
    writer.event(0, &Event::Call { frame: 0, caller: None, function: "f", inputs: &inputs, types: &types });
    let none = |w: &mut ValueWriter<'_>| w.null();
    let session = |w: &mut ValueWriter<'_>| w.string("SIM::DMM0");
    writer.event(0, &Event::Ran { frame: 0, node: "dmm", loops: &[], iter: &[], out: &[("dmm", &session), ("e1", &none)], types: &[] });
    for iteration in 0..3u64 {
        let raw = move |w: &mut ValueWriter<'_>| w.f64(iteration as f64 + 0.5, None);
        writer.event(0, &Event::Ran { frame: 0, node: "raw", loops: &["readings"], iter: &[iteration], out: &[("raw", &raw)], types: &[] });
        let even = iteration % 2 == 0;
        writer.event(0, &Event::Arm { frame: 0, node: "v", loops: &["readings"], iter: &[iteration], arm: if even { "true" } else { "false" } });
        let text = |w: &mut ValueWriter<'_>| w.string("even");
        let odd = move |w: &mut ValueWriter<'_>| w.i64(iteration as i64, None);
        if even {
            writer.event(
                0,
                &Event::Ran { frame: 0, node: "x", loops: &["readings"], iter: &[iteration], out: &[("x", &text)], types: &[("x", "string")] },
            );
        } else {
            writer
                .event(0, &Event::Ran { frame: 0, node: "x", loops: &["readings"], iter: &[iteration], out: &[("x", &odd)], types: &[("x", "i32")] });
        }
    }
    let readings = |w: &mut ValueWriter<'_>| {
        let count = w.begin_array(3);
        for index in 0..count {
            w.f64(index as f64 + 0.5, None);
        }
        w.end();
    };
    writer.event(0, &Event::Ran { frame: 0, node: "readings", loops: &[], iter: &[], out: &[("readings", &readings)], types: &[] });
    writer.end(1_000_000, "ok");
    let bytes = writer.into_inner();

    let mut reader = Reader::new(expected(), ReadLimits::default());
    assert!(reader.feed(&bytes));
    let trace = reader.finish();
    assert_eq!(trace.header.as_ref().map(|header| header.files.clone()), Ok(vec![("f.lcf".to_owned(), "ab".to_owned())]));
    assert_eq!((trace.events, trace.end.as_deref(), trace.cut(), trace.ignored), (13, Some("ok"), false, 0));
    assert_eq!(trace.input(0), Some(r#""SIM::DMM0""#));
    assert_eq!(trace.input(1), Some(r#""220.0 V""#));
    assert_eq!(trace.value(DMM, 0, &[]), Some(r#""SIM::DMM0""#));
    assert_eq!(trace.value(DMM, 1, &[]), Some("null"));
    assert_eq!(trace.values(RAW, 0).collect::<Vec<_>>(), [(&[0][..], "0.5"), (&[1][..], "1.5"), (&[2][..], "2.5")]);
    assert_eq!(trace.value(READINGS, 0, &[]), Some("[0.5,1.5,2.5]"));
    assert_eq!(trace.arm(V, &[1]), Some("false"));
    // The two `x` are told apart by the arm taken before them.
    assert_eq!(trace.value(X_TRUE, 0, &[0]), Some(r#""even""#));
    assert_eq!(trace.value(X_FALSE, 0, &[1]), Some("1"));
    assert_eq!(trace.value(X_TRUE, 0, &[1]), None);
    assert_eq!(trace.iterations(READINGS, &[]), 3);
    assert_eq!(trace.iterations(RAW, &[]), 0, "not a loop");
    assert!(!trace.skipped(RAW, &[0]));
}

/// Each line is one case of cli.md 7.4 that fits nothing: counted, the first one's line given, the rest shown.
#[test]
fn what_fits_nothing_is_counted_and_the_rest_shown() {
    let lines = [
        FIRST,
        r#"{"seq":1,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"e1":null}}"#, // before the call of frame 0
        CALL,
        r#"{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"nowhere","out":{"a":1}}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"e1":null,"other":1}}"#, // a wire it does not have
        r#"{"seq":4,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[0],"out":{"raw":"1 V"},"types":{"raw":"f64[V]"}}"#,
        r#"{"seq":5,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","out":{"raw":1.0}}"#, // no iteration in a loop
        r#"{"seq":6,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[9223372036854775808],"out":{"raw":1.0}}"#,
        r#"{"seq":7,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[1],"out":{"raw":1.0}}"#,
        r#"{"seq":8,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[1],"out":{"raw":2.0}}"#, // given already
        r#"{"seq":9,"t_ms":0.0,"event":"ran","frame":0,"node":"x","loops":["readings"],"iter":[1],"out":{"x":"a"}}"#,     // no arm taken yet
        r#"{"seq":10,"t_ms":0.0,"event":"ran","frame":3,"node":"y","out":{"y":1}}"#, // a frame that no call opened
        r#"{"seq":11,"t_ms":0.0,"event":"arm","frame":0,"node":"v","loops":["readings"],"iter":[1],"arm":"true"}"#,
        r#"{"seq":12,"t_ms":0.0,"event":"arm","frame":0,"node":"v","loops":["readings"],"iter":[1],"arm":"false"}"#, // given already
        r#"{"seq":13,"t_ms":0.0,"event":"other","frame":0}"#,
        r#"{"seq":14,"t_ms":0.0,"event":"call","frame":0,"function":"f","in":[],"types":{}}"#, // frame 0 again
        r#"{"seq":15,"t_ms":0.0,"event":"end","status":"ok"}"#,
    ];
    let trace = read(&lines);
    assert_eq!(trace.stop, None);
    assert_eq!((trace.ignored, trace.first_ignored), (12, Some(2)));
    assert_eq!(trace.value(DMM, 1, &[]), Some("null"), "the wire it has is kept");
    assert_eq!(trace.value(RAW, 0, &[1]), Some("1.0"), "the first value given");
    assert_eq!(trace.arm(V, &[1]), Some("true"));
    assert_eq!(trace.values(RAW, 0).count(), 1);
    assert_eq!(trace.end.as_deref(), Some("ok"));
}

#[test]
fn a_value_longer_than_a_run_writes_fits_nothing() {
    let long = format!(
        r#"{{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"v","loops":["readings"],"iter":[0],"out":{{"v":"{}"}}}}"#,
        "a".repeat(VALUE_BUDGET)
    );
    let trace = read(&[FIRST, CALL, &long]);
    assert_eq!((trace.ignored, trace.value(V, 0, &[0])), (1, None));
}

/// Lines of other frames are read only as far as their frame: what follows it is never decoded.
#[test]
fn other_frames_are_decoded_only_as_far_as_their_frame() {
    let lines = [
        FIRST,
        CALL,
        r#"{"seq":2,"t_ms":0.0,"event":"call","frame":1,"parent":0,"node":"dmm","in": this is never read"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":1, not JSON after the frame"#,
        r#"{"seq":4,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"e1":null}}"#,
    ];
    let trace = read(&lines);
    assert_eq!((trace.stop, trace.ignored), (None, 0));
    assert_eq!(trace.value(DMM, 1, &[]), Some("null"));
}

#[test]
fn a_first_line_that_does_not_read_gives_no_values() {
    for (first, problem) in [
        ("", HeaderProblem::Unreadable),
        ("not json", HeaderProblem::Unreadable),
        ("[1]", HeaderProblem::Unreadable),
        (r#"{"schema":"labcraft-trace/1","run":"r1","function":"f","mode":"deterministic"}"#, HeaderProblem::Unreadable),
        (r#"{"schema":"labcraft-trace/1","run":"r1","function":"f","mode":"deterministic","files":{}}"#, HeaderProblem::Unreadable),
        (r#"{"schema":"labcraft-run/1"}"#, HeaderProblem::OtherSchema),
        (r#"{"schema":"labcraft-trace/1.1"}"#, HeaderProblem::OtherSchema),
        (r#"{"schema":"labcraft-trace/2"}"#, HeaderProblem::LaterVersion),
        (r#"{"schema":"labcraft-trace/99999999999999999999999"}"#, HeaderProblem::LaterVersion),
    ] {
        let trace = read(&[first, CALL]);
        assert_eq!(trace.header, Err(problem), "{first}");
        assert_eq!((trace.input(0), trace.stop, trace.events), (None, None, 0), "{first}");
    }
    let mut reader = Reader::new(expected(), ReadLimits::default());
    assert!(!reader.feed(b"\xff\xfe\n"));
    assert_eq!(reader.finish().header, Err(HeaderProblem::Unreadable), "not UTF-8");
    assert_eq!(Reader::new(expected(), ReadLimits::default()).finish().header, Err(HeaderProblem::Unreadable), "an empty file");
}

#[test]
fn the_trace_of_another_function_gives_no_values() {
    let first = FIRST.replace(r#""function":"f""#, r#""function":"g""#);
    let trace = read(&[&first, CALL]);
    assert!(trace.other_function);
    assert_eq!((trace.input(0), trace.events), (None, 0));
    assert_eq!(trace.header.map(|header| header.function), Ok("g".to_owned()));
}

/// A line that does not read ends the values there, with its line; what came before stands.
#[test]
fn a_line_that_does_not_read_ends_the_values_there() {
    let dmm = r#"{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"e1":null}}"#;
    for bad in [
        "not json",
        "",
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":-1}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[-1],"out":{}}"#,
        r#"{"seq":-3,"t_ms":0.0,"event":"end","status":"ok"}"#,
        r#"{"t_ms":0.0,"event":"end","status":"ok"}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"end"}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm"}"#,
        r#"{"seq":3,"t_ms":0.0,"seq":4,"event":"end","status":"ok"}"#,
        r#"{"seq":3,"t_ms":"0.0","event":"end","status":"ok"}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{"e1":nul}}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"end","status":"ok"} trailing"#,
    ] {
        let trace = read(&[FIRST, CALL, dmm, bad, r#"{"seq":4,"t_ms":0.0,"event":"end","status":"ok"}"#]);
        assert_eq!(trace.stop, Some(Stop { line: 4, limit: Limit::Unreadable }), "{bad}");
        assert_eq!((trace.events, trace.end.as_deref()), (2, None), "{bad}");
        assert_eq!(trace.value(DMM, 1, &[]), Some("null"), "{bad}");
    }
    let deep = format!(r#"{{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{{"e1":{}1{}}}}}"#, "[".repeat(300), "]".repeat(300));
    assert_eq!(read(&[FIRST, CALL, &deep]).stop, Some(Stop { line: 3, limit: Limit::Unreadable }), "nested past 256 levels");
}

#[test]
fn the_limits_of_reading_stop_it_with_their_line() {
    let event = r#"{"seq":2,"t_ms":0.0,"event":"arm","frame":0,"node":"v","loops":["readings"],"iter":[0],"arm":"true"}"#;
    let line_limit = ReadLimits { line: CALL.len(), ..ReadLimits::default() };
    let long = event.replace("\"true\"", &format!("\"{}\"", "t".repeat(CALL.len())));
    let trace = read_with(&[FIRST, CALL, &long], line_limit);
    assert_eq!(trace.stop, Some(Stop { line: 3, limit: Limit::Line }));
    assert_eq!(trace.input(0), Some(r#""SIM::DMM0""#));

    let lines_limit = ReadLimits { lines: 2, ..ReadLimits::default() };
    let trace = read_with(&[FIRST, CALL, event, event], lines_limit);
    assert_eq!(trace.stop, Some(Stop { line: 4, limit: Limit::Lines }));
    assert_eq!(trace.arm(V, &[0]), Some("true"));

    // The file stops in the middle of the third line.
    let file_limit = ReadLimits { file: (FIRST.len() + CALL.len() + 2 + 10) as u64, ..ReadLimits::default() };
    let trace = read_with(&[FIRST, CALL, event], file_limit);
    assert_eq!(trace.stop, Some(Stop { line: 3, limit: Limit::File }));
    assert_eq!((trace.events, trace.arm(V, &[0])), (1, None));

    // What it keeps: the first line, the call and the first arm fit; the second arm passes the bound.
    let mut kept = Reader::new(expected(), ReadLimits::default());
    kept.feed(format!("{FIRST}\n{CALL}\n{event}\n").as_bytes());
    let bound = kept.kept;
    let second = event.replace("[0]", "[1]");
    let trace = read_with(&[FIRST, CALL, event, &second], ReadLimits { kept: bound, ..ReadLimits::default() });
    assert_eq!(trace.stop, Some(Stop { line: 4, limit: Limit::Kept }));
    assert_eq!((trace.arm(V, &[0]), trace.arm(V, &[1])), (Some("true"), None));
}

/// The same bytes give the same trace however they are cut, and a last line without its end is read.
#[test]
fn bytes_can_come_in_any_parts() {
    let lines = [
        FIRST,
        CALL,
        r#"{"seq":2,"t_ms":0.0,"event":"arm","frame":0,"node":"v","loops":["readings"],"iter":[0],"arm":"true"}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"end","status":"ok"}"#,
    ];
    let whole = read(&lines);
    let text = lines.join("\n");
    let mut reader = Reader::new(expected(), ReadLimits::default());
    for byte in text.as_bytes().chunks(1) {
        reader.feed(byte);
    }
    let by_bytes = reader.finish();
    assert_eq!(by_bytes, whole);
    assert_eq!(by_bytes.end.as_deref(), Some("ok"), "the last line has no end of line");
}

/// A trace built to cost the most: a huge iteration, many levels of `iter`, a million keys, a first line of many
/// files. Each costs its length: nothing is allocated for an iteration number.
#[test]
fn a_hostile_trace_costs_its_length() {
    let levels = format!(
        r#"{{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[{}],"out":{{"raw":1.0}}}}"#,
        vec!["18446744073709551615"; 100_000].join(",")
    );
    let keys: Vec<String> = (0..100_000).map(|index| format!(r#""k{index}":{index}"#)).collect();
    let many = format!(r#"{{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{{{}}}}}"#, keys.join(","));
    let names: Vec<String> = (0..100_000).map(|index| format!(r#""k{index}":"t""#)).collect();
    let types = format!(r#"{{"seq":4,"t_ms":0.0,"event":"ran","frame":0,"node":"dmm","out":{{"e1":null}},"types":{{{}}}}}"#, names.join(","));
    let trace = read(&[FIRST, CALL, &levels, &many, &types]);
    assert_eq!(trace.stop, None);
    assert_eq!(trace.ignored, 2);
    assert_eq!(trace.value(DMM, 1, &[]), Some("null"));
}

/// The lines that the writer shortens to stay within 16 MiB are read: the trace is cut, and what follows them stands.
#[test]
fn the_lines_the_writer_shortens_are_read() {
    let header = Header { run: "r1", function: "f", mode: "deterministic", files: &[] };
    let limits = crate::write::WriteLimits { line: 300, ..crate::write::WriteLimits::default() };
    let mut writer = TraceWriter::with_limits(Vec::new(), &header, limits);
    let long = "n".repeat(100);
    let none = |w: &mut ValueWriter<'_>| w.null();
    let inputs: Vec<Input<'_>> = (0..20).map(|_| Input { name: &long, ty: "string", value: &none }).collect();
    writer.event(0, &Event::Call { frame: 0, caller: None, function: "f", inputs: &inputs, types: &[("e1", "error")] });
    let out: Vec<(&str, &dyn crate::value::WriteValue)> = (0..5).map(|_| (long.as_str(), &none as &dyn crate::value::WriteValue)).collect();
    writer.event(0, &Event::Ran { frame: 0, node: "raw", loops: &[], iter: &[0], out: &out, types: &[] });
    writer.event(0, &Event::Ran { frame: 0, node: "dmm", loops: &[], iter: &[], out: &[("e1", &none)], types: &[] });
    writer.end(0, "ok");
    let bytes = writer.into_inner();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.lines().nth(1).unwrap().ends_with(r#""frame":0,"truncated":true}"#), "{text}");
    let mut reader = Reader::new(expected(), ReadLimits::default());
    reader.feed(&bytes);
    let trace = reader.finish();
    assert_eq!((trace.stop, trace.end.as_deref(), trace.truncated, trace.ignored), (None, Some("ok"), true, 0));
    // `e1` has no type, since the call lost its `types`: a cut value, dropped without a word.
    assert_eq!(trace.value(DMM, 1, &[]), None);
}

/// Thousands of nodes of one name, each in an arm of its own: a name is resolved in a few lookups, whatever their number.
#[test]
fn many_nodes_of_one_name_are_resolved_by_the_arms_taken() {
    let mut expected = Expected { function: "f".into(), ..Expected::default() };
    for index in 0..5_000u32 {
        expected.nodes.push(node(&format!("m{index}"), &[], &[], &[]));
    }
    for index in 0..5_000u32 {
        expected.nodes.push(node("x", &[], &[(index, "true")], &[("x", "i32")]));
    }
    let mut lines = vec![FIRST.to_owned(), CALL.replace(r#""v":"string""#, r#""x":"i32""#)];
    lines.push(r#"{"seq":2,"t_ms":0.0,"event":"arm","frame":0,"node":"m4321","arm":"true"}"#.to_owned());
    for seq in 0..20_000 {
        lines.push(format!(r#"{{"seq":{seq},"t_ms":0.0,"event":"ran","frame":0,"node":"x","out":{{"x":{seq}}}}}"#));
    }
    lines.push(r#"{"seq":2,"t_ms":0.0,"event":"arm","frame":0,"node":"m7","arm":"true"}"#.to_owned());
    lines.push(r#"{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"x","out":{"x":-1}}"#.to_owned());
    let mut reader = Reader::new(expected, ReadLimits::default());
    reader.feed(lines.join("\n").as_bytes());
    let trace = reader.finish();
    assert_eq!(trace.value(5_000 + 4321, 0, &[]), Some("0"), "the arm taken tells it apart");
    // The call gives two inputs that this function does not have, the other 19,999 are values given already, and the
    // last one has two nodes whose arms were taken: 1 + 19,999 + 1.
    assert_eq!(trace.ignored, 20_001);
    assert_eq!(trace.stop, None);
}

#[test]
fn values_within_an_outer_iteration() {
    let lines = [
        FIRST,
        CALL,
        r#"{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[1],"out":{"raw":1.0}}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"raw","loops":["readings"],"iter":[0],"out":{"raw":0.0}}"#,
    ];
    let trace = read(&lines);
    assert_eq!(trace.values_within(RAW, 0, &[1]).collect::<Vec<_>>(), [(&[1][..], "1.0")]);
    assert_eq!(trace.values_within(RAW, 0, &[]).count(), 2);
    assert_eq!(trace.iterations(READINGS, &[]), 2, "in any order");
}

/// What a line of another frame holds after its frame is not read, whatever the order of its fields; a first line of a
/// later version is one whatever its shape; a byte order mark is not part of the first line.
#[test]
fn the_order_of_fields_and_the_shape_of_a_later_version_do_not_matter() {
    let lines = [
        FIRST,
        CALL,
        r#"{"event":"call","frame":1,"seq":2,"t_ms":0.0}"#,
        r#"{"frame":0,"node":"dmm","out":{"e1":null},"t_ms":0.0,"event":"ran","seq":3}"#,
    ];
    let trace = read(&lines);
    assert_eq!((trace.stop, trace.ignored, trace.value(DMM, 1, &[])), (None, 0, Some("null")));
    let later = read(&[r#"{"files":["a.lcf"],"schema":"labcraft-trace/2"}"#]);
    assert_eq!(later.header, Err(HeaderProblem::LaterVersion));
    let bom = format!("\u{feff}{FIRST}");
    assert!(read(&[&bom, CALL]).header.is_ok());
}

/// Two loops that bind the same names (the two branches of `two_benches`): each event names its loops (`loops`,
/// `docs/commands.md`, 8), which tell the two `m` apart; loops that no node has fit nothing.
#[test]
fn the_names_of_the_loops_tell_two_nodes_apart() {
    let expected = Expected {
        function: "f".into(),
        inputs: Vec::new(),
        nodes: vec![
            node("n_a", &[], &[], &[("n_a", "f64")]),
            node("m", &[0], &[], &[("m", "f64")]),
            node("n_b", &[], &[], &[("n_b", "f64")]),
            node("m", &[2], &[], &[("m", "f64")]),
        ],
    };
    let call = r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"f","in":[],"types":{"n_a":"f64","n_b":"f64","m":"f64"}}"#;
    let lines = [
        FIRST,
        call,
        r#"{"seq":2,"t_ms":0.0,"event":"ran","frame":0,"node":"m","loops":["n_a"],"iter":[0],"out":{"m":1.0}}"#,
        r#"{"seq":3,"t_ms":0.0,"event":"ran","frame":0,"node":"m","loops":["n_b"],"iter":[0],"out":{"m":2.0}}"#,
        r#"{"seq":4,"t_ms":0.0,"event":"ran","frame":0,"node":"m","loops":["n_c"],"iter":[0],"out":{"m":3.0}}"#,
        r#"{"seq":5,"t_ms":0.0,"event":"ran","frame":0,"node":"m","iter":[1],"out":{"m":4.0}}"#,
        r#"{"seq":6,"t_ms":0.0,"event":"end","status":"ok"}"#,
    ];
    let mut reader = Reader::new(expected, ReadLimits::default());
    reader.feed(lines.join("\n").as_bytes());
    let trace = reader.finish();
    assert_eq!((trace.value(1, 0, &[0]), trace.value(3, 0, &[0])), (Some("1.0"), Some("2.0")));
    assert_eq!((trace.ignored, trace.first_ignored), (2, Some(5)));
    assert_eq!((trace.iterations(0, &[]), trace.iterations(2, &[])), (1, 1));
}
