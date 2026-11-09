use super::*;

fn written(budget: usize, write: impl Fn(&mut ValueWriter<'_>)) -> (String, Written) {
    let mut out = String::new();
    let mut writer = ValueWriter::new(&mut out, budget);
    write(&mut writer);
    let result = writer.finish();
    (out, result)
}

fn text(write: impl Fn(&mut ValueWriter<'_>)) -> String {
    written(VALUE_BUDGET, write).0
}

/// The forms of commands.md 5, one per row of its table.
#[test]
fn each_type_has_its_form() {
    assert_eq!(text(|w| w.bool(true)), "true");
    assert_eq!(text(|w| w.null()), "null");
    assert_eq!(text(|w| w.i64(12, None)), "12");
    assert_eq!(text(|w| w.i64(9_007_199_254_740_993, None)), r#""9007199254740993""#);
    assert_eq!(text(|w| w.u64(u64::MAX, None)), r#""18446744073709551615""#);
    assert_eq!(text(|w| w.i64(-3, Some("mV"))), r#""-3 mV""#);
    assert_eq!(text(|w| w.f64(0.5, None)), "0.5");
    assert_eq!(text(|w| w.f64(12.0, None)), "12.0");
    assert_eq!(text(|w| w.f64(f64::NAN, None)), r#""nan""#);
    assert_eq!(text(|w| w.f64(f64::NEG_INFINITY, Some("V"))), r#""-inf V""#);
    assert_eq!(text(|w| w.f64(230.03, Some("V"))), r#""230.03 V""#);
    assert_eq!(text(|w| w.f64(-1.5e-3, Some("A"))), r#""-0.0015 A""#);
    assert_eq!(text(|w| w.f32(0.1, None)), "0.1");
    assert_eq!(text(|w| w.duration(100_000_000)), r#""0.1 s""#);
    assert_eq!(text(|w| w.timestamp(1_794_139_200_000_000_250)), r#""2026-11-08T12:00:00.000000250Z""#);
    assert_eq!(text(|w| w.complex(1.0, 0.0, false, Some("V"))), r#"{"re":"1.0 V","im":"0.0 V"}"#);
    assert_eq!(text(|w| w.complex(0.1, f64::NAN, true, None)), r#"{"re":0.1,"im":"nan"}"#);
    assert_eq!(text(|w| w.string("MEAS:VOLT:DC?")), r#""MEAS:VOLT:DC?""#);
    assert_eq!(text(|w| w.string("a\"b\\\n\u{1}é")), r#""a\"b\\\n\u0001é""#);
    assert_eq!(text(|w| w.bytes(b"foo")), r#"{"base64":"Zm9v"}"#);
    assert_eq!(text(|w| w.variant("Verdict.Pass")), r#""Verdict.Pass""#);
    let reading = text(|w| {
        let count = w.begin_variant("Reading.Value", 1);
        assert_eq!(count, 1);
        w.f64(230.0, Some("V"));
        w.end_variant();
    });
    assert_eq!(reading, r#"{"Reading.Value":["230.0 V"]}"#);
    let waveform = text(|w| {
        w.begin_object();
        w.key("t0");
        w.timestamp(0);
        w.key("dt");
        w.duration(1_000_000);
        w.key("samples");
        let count = w.begin_array(2);
        for sample in 0..count {
            w.f64(sample as f64, None);
        }
        w.end();
        w.end();
    });
    assert_eq!(waveform, r#"{"t0":"1970-01-01T00:00:00.000000000Z","dt":"0.001 s","samples":[0.0,1.0]}"#);
    let opaque = text(|w| {
        w.begin_object();
        w.key("opaque");
        w.string("visa.session");
        w.key("resource");
        w.string("SIM::DMM0");
        w.key("open");
        w.bool(false);
        w.end();
    });
    assert_eq!(opaque, r#"{"opaque":"visa.session","resource":"SIM::DMM0","open":false}"#);
}

#[test]
fn an_error_lists_at_most_32_others_flat() {
    let place = Place { function: "scpi.query_f64", node: "text", file: "<labcraft>/scpi/query_f64.lcf", line: 12 };
    let caller = Place { function: "check_voltage", node: "raw", file: "check_voltage.lcf", line: 15 };
    let inner = [ErrorValue { category: "x", code: 0, message: "", origin: None, others: &[] }];
    let other = ErrorValue { category: "io.timeout", code: 1, message: "m", origin: None, others: &inner };
    let others = vec![other; 34];
    let error = ErrorValue {
        category: "io.timeout",
        code: -1_073_807_339,
        message: "no answer",
        origin: Some(Origin { place, callers: std::slice::from_ref(&caller) }),
        others: &others,
    };
    let json: serde_json::Value = serde_json::from_str(&text(|w| w.error(&error))).unwrap();
    assert_eq!(json["code"], -1_073_807_339);
    assert_eq!(json["origin"]["file"], "<labcraft>/scpi/query_f64.lcf");
    assert_eq!(json["origin"]["callers"][0]["line"], 15);
    assert_eq!(json["others"].as_array().unwrap().len(), 32);
    assert_eq!(json["others_more"], 2);
    assert!(json["others"][0].get("others").is_none(), "flat");
    let none = ErrorValue { others: &[], origin: None, ..error.clone() };
    assert_eq!(text(|w| w.error(&none)), r#"{"category":"io.timeout","code":-1073807339,"message":"no answer","others":[]}"#);
}

#[test]
fn a_long_array_shows_its_length_and_head_at_every_level() {
    let whole = text(|w| {
        let count = w.begin_array(64);
        assert_eq!(count, 64);
        for _ in 0..count {
            w.i64(1, None);
        }
        w.end();
    });
    assert_eq!(whole, format!("[{}]", vec!["1"; 64].join(",")));
    let cut = text(|w| {
        let outer = w.begin_array(100_000);
        assert_eq!(outer, 16);
        for row in 0..outer {
            let inner = w.begin_array(65);
            for column in 0..inner {
                w.i64((row * 100 + column) as i64, None);
            }
            w.end();
        }
        w.end();
    });
    let json: serde_json::Value = serde_json::from_str(&cut).unwrap();
    assert_eq!(json["$len"], 100_000);
    assert_eq!(json["$head"].as_array().unwrap().len(), 16);
    assert_eq!(json["$head"][3]["$len"], 65);
    assert_eq!(json["$head"][3]["$head"][15], 315);
}

#[test]
fn a_long_string_or_bytes_show_their_length_and_head() {
    let thousand = "é".repeat(1000);
    assert_eq!(text(|w| w.string(&thousand)), format!("\"{thousand}\""));
    let longer = format!("{thousand}éé");
    assert_eq!(text(|w| w.string(&longer)), format!(r#"{{"$len":1002,"$head":"{thousand}"}}"#));
    let bytes = vec![0u8; 1001];
    let json: serde_json::Value = serde_json::from_str(&text(|w| w.bytes(&bytes))).unwrap();
    assert_eq!(json["$len"], 1001);
    // 1,000 zero bytes: 333 groups of three and one left, 1,336 symbols.
    assert_eq!(json["$head"]["base64"].as_str().unwrap().len(), 1336);
}

/// Each `"é…é"` of 1,000 code points takes 2,002 bytes, and 2,003 with its comma. The 33rd would end past
/// 65,536 - 14 (the reserve) - 1 (`]`), since 1 + 32 × 2,003 - 1 + 2,003 = 66,099: 32 fit, then the mark.
#[test]
fn a_value_past_its_budget_ends_with_a_cut() {
    let thousand = "é".repeat(1000);
    let (out, result) = written(VALUE_BUDGET, |w| {
        let count = w.begin_array(40);
        for _ in 0..count {
            w.string(&thousand);
        }
        w.end();
    });
    assert!(out.len() <= VALUE_BUDGET);
    assert_eq!(result, Written { cut: true, spent: true });
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 33);
    assert_eq!(items[31], thousand.as_str());
    assert_eq!(items[32], serde_json::json!({ "$cut": true }));
}

/// A budget of 40: `{"aaaaaaaaaa":1` takes 15 bytes, and the next key, 14 more, would leave less than the reserve
/// and the `}`: 15 + 14 + 14 + 1 = 44.
#[test]
fn a_field_that_does_not_fit_ends_its_object_with_a_cut() {
    let (out, result) = written(40, |w| {
        w.begin_object();
        w.key("aaaaaaaaaa");
        w.i64(1, None);
        w.key("bbbbbbbbbb");
        w.i64(2, None);
        w.end();
    });
    assert_eq!(out, r#"{"aaaaaaaaaa":1,"$cut":true}"#);
    assert!(result.spent);
    let (out, _) = written(5, |w| w.string("hello world"));
    assert_eq!(out, CUT, "a value larger than its budget");
}

/// The top array is level 1; 62 more fit inside it, so that the mark is level 64. What follows the deep part is
/// still written.
#[test]
fn depth_is_bounded_and_cuts_only_what_is_too_deep() {
    let (out, result) = written(VALUE_BUDGET, |w| {
        w.begin_array(2);
        for _ in 0..100 {
            w.begin_array(1);
        }
        w.bool(true);
        for _ in 0..100 {
            w.end();
        }
        w.i64(1, None);
        w.end();
    });
    assert_eq!(out, format!("[{}{CUT}{},1]", "[".repeat(62), "]".repeat(62)));
    assert_eq!(result, Written { cut: true, spent: false });
    // The levels of `$len` and `$head` count: two for a long array.
    let (out, _) = written(VALUE_BUDGET, |w| {
        for _ in 0..62 {
            w.begin_array(1);
        }
        w.begin_array(100);
        for _ in 0..63 {
            w.end();
        }
    });
    assert_eq!(out, format!("{}{CUT}{}", "[".repeat(62), "]".repeat(62)));
}

#[test]
fn what_the_caller_leaves_open_still_reads() {
    let (out, _) = written(VALUE_BUDGET, |w| {
        w.begin_array(3);
        w.i64(1, None);
    });
    assert_eq!(out, "[1]");
    let (out, result) = written(VALUE_BUDGET, |w| {
        w.begin_object();
        w.key("a");
        w.end();
    });
    assert_eq!(out, format!(r#"{{"a":{CUT}}}"#));
    assert!(result.cut);
    assert_eq!(written(VALUE_BUDGET, |_| {}).0, CUT, "no value at all");
    assert_eq!(
        written(VALUE_BUDGET, |w| {
            w.i64(1, None);
            w.i64(2, None);
        })
        .0,
        "1",
        "one value only"
    );
}

/// The marks of a part too deep stay within the budget: 63 arrays deep, strings that nearly fill it, then marks of
/// arrays too deep, side by side, until the budget is spent.
#[test]
fn marks_of_depth_stay_within_the_budget() {
    let thousand = "a".repeat(1000);
    let (out, result) = written(VALUE_BUDGET, |w| {
        for _ in 0..63 {
            w.begin_array(1);
        }
        for _ in 0..64 {
            w.string(&thousand);
        }
        for _ in 0..200 {
            w.begin_array(1);
            w.end();
        }
        for _ in 0..63 {
            w.end();
        }
    });
    assert!(out.len() <= VALUE_BUDGET, "{}", out.len());
    assert_eq!(result, Written { cut: true, spent: true });
    serde_json::from_str::<serde_json::Value>(&out).unwrap();
}
