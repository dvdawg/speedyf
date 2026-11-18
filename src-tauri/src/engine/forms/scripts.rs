//! Recognising the standard Acrobat form helpers in field scripts.
//!
//! Field scripts are JavaScript, and SpeedyF never runs JavaScript. Most of
//! what a form's scripts do, though, is call one of a dozen helpers that
//! Acrobat ships in AForm.js - format this field as currency, take only
//! digits, sum these three fields - with literal arguments. Those calls are
//! recognised here by their shape and handed to the frontend as data, which
//! implements each helper itself. Anything else - a custom function, an
//! `if`, a second statement - is counted as a script not run and left alone.
//!
//! The parser accepts exactly one call statement: an identifier, literal
//! arguments (numbers, strings, booleans, `new Array(...)` or `[...]` of
//! those), an optional trailing semicolon, whitespace and comments anywhere
//! between tokens. That is what Acrobat itself writes when an author picks a
//! format from the field properties dialog.

use serde::Serialize;

/// How a field's value is shown, or what typing it accepts.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FieldFormat {
    /// AFNumber_Format / AFNumber_Keystroke.
    Number {
        decimals: u32,
        /// 0 `1,234.56`, 1 `1234.56`, 2 `1.234,56`, 3 `1234,56`, 4 `1'234.56`.
        sep_style: u8,
        /// 0 minus, 1 red, 2 parentheses, 3 red parentheses.
        neg_style: u8,
        currency: String,
        prepend: bool,
    },
    /// AFPercent_Format / AFPercent_Keystroke.
    Percent {
        decimals: u32,
        sep_style: u8,
        /// The percent sign goes before the number.
        prepend: bool,
    },
    /// AFDate_FormatEx / AFDate_KeystrokeEx, and AFDate_Format(n) resolved
    /// to its format string. An empty format means the reader's own.
    Date { format: String },
    /// AFTime_Format(n) / AFTime_Keystroke(n), resolved to a format string.
    Time { format: String },
    /// AFSpecial_Format / AFSpecial_Keystroke: 0 zip, 1 zip+4, 2 phone, 3 SSN.
    Special { style: u8 },
    /// AFSpecial_KeystrokeEx: 9 digit, A letter, O letter or digit, X any.
    Mask { mask: String },
}

/// AFRange_Validate: the value must be >= `min` and/or <= `max`.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FieldRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// AFSimple_Calculate: the field is `op` over the values of `fields`.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FieldCalc {
    /// SUM, PRD, AVG, MIN or MAX.
    pub op: String,
    pub fields: Vec<String>,
}

/// What a field's keystroke, format, validate and calculate scripts were
/// recognised as. A slot is empty when the script is absent or was not one
/// of the standard helpers.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FieldBehaviour {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keystroke: Option<FieldFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<FieldFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<FieldRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate: Option<FieldCalc>,
}

impl FieldBehaviour {
    pub fn is_empty(&self) -> bool {
        self.keystroke.is_none()
            && self.format.is_none()
            && self.validate.is_none()
            && self.calculate.is_none()
    }

    /// Record the script for additional-action `event` (K, F, V or C).
    /// Returns whether it was recognised.
    pub fn take(&mut self, event: &[u8], source: &str) -> bool {
        let Some(call) = parse_call(source) else {
            return false;
        };
        match event {
            b"K" => set(&mut self.keystroke, keystroke(&call)),
            b"F" => set(&mut self.format, format(&call)),
            b"V" => set(&mut self.validate, validate(&call)),
            b"C" => set(&mut self.calculate, calculate(&call)),
            _ => false,
        }
    }
}

fn set<T>(slot: &mut Option<T>, value: Option<T>) -> bool {
    match value {
        Some(v) => {
            *slot = Some(v);
            true
        }
        None => false,
    }
}

/// How many JavaScript actions a form carries, and how many were taken for
/// standard helpers. Everything else is never run.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptSummary {
    /// Field and widget actions that are JavaScript.
    pub total: u32,
    /// Of those, the ones recognised as standard helpers.
    pub recognised: u32,
    /// Document-level scripts (the /JavaScript name tree).
    pub document: u32,
}

/// Acrobat's AFDate_Format(n) table.
const DATE_FORMATS: [&str; 14] = [
    "m/d",
    "m/d/yy",
    "mm/dd/yy",
    "mm/yy",
    "d-mmm",
    "d-mmm-yy",
    "dd-mmm-yy",
    "yy-mm-dd",
    "mmm-yy",
    "mmmm-yy",
    "mmm d, yyyy",
    "mmmm d, yyyy",
    "m/d/yy h:MM tt",
    "m/d/yy HH:MM",
];

/// Acrobat's AFTime_Format(n) table.
const TIME_FORMATS: [&str; 4] = ["HH:MM", "h:MM tt", "HH:MM:ss", "h:MM:ss tt"];

/// A literal argument.
#[derive(Clone, Debug, PartialEq)]
pub enum JsValue {
    Number(f64),
    Str(String),
    Bool(bool),
    Array(Vec<JsValue>),
}

impl JsValue {
    fn number(&self) -> Option<f64> {
        match self {
            JsValue::Number(n) => Some(*n),
            JsValue::Str(s) => s.trim().parse().ok(),
            JsValue::Bool(b) => Some(f64::from(u8::from(*b))),
            JsValue::Array(_) => None,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::Str(s) => !s.is_empty(),
            JsValue::Bool(b) => *b,
            JsValue::Array(_) => true,
        }
    }

    fn string(&self) -> Option<String> {
        match self {
            JsValue::Str(s) => Some(s.clone()),
            JsValue::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

/// A recognised call: helper name and literal arguments.
#[derive(Debug, PartialEq)]
pub struct Call {
    pub name: String,
    pub args: Vec<JsValue>,
}

impl Call {
    fn arg(&self, i: usize) -> Option<&JsValue> {
        self.args.get(i)
    }
    fn int(&self, i: usize, default: i64) -> Option<i64> {
        match self.arg(i) {
            None => Some(default),
            Some(v) => v
                .number()
                .filter(|n| n.is_finite())
                .map(|n| n.trunc() as i64),
        }
    }
    fn flag(&self, i: usize, default: bool) -> bool {
        self.arg(i).map_or(default, JsValue::truthy)
    }
    fn text(&self, i: usize) -> Option<String> {
        match self.arg(i) {
            None => Some(String::new()),
            Some(v) => v.string(),
        }
    }
}

fn sep_style(call: &Call, i: usize) -> Option<u8> {
    call.int(i, 0).map(|n| n.clamp(0, 4) as u8)
}

fn decimals(call: &Call) -> Option<u32> {
    call.int(0, 0)
        .filter(|n| *n >= 0 && *n <= 20)
        .map(|n| n as u32)
}

fn number_format(call: &Call) -> Option<FieldFormat> {
    Some(FieldFormat::Number {
        decimals: decimals(call)?,
        sep_style: sep_style(call, 1)?,
        neg_style: call.int(2, 0)?.clamp(0, 3) as u8,
        currency: call.text(4)?,
        prepend: call.flag(5, true),
    })
}

fn percent_format(call: &Call) -> Option<FieldFormat> {
    Some(FieldFormat::Percent {
        decimals: decimals(call)?,
        sep_style: sep_style(call, 1)?,
        prepend: call.flag(2, false),
    })
}

fn table(call: &Call, list: &[&str]) -> Option<String> {
    let n = call.int(0, 0)?;
    usize::try_from(n)
        .ok()
        .and_then(|i| list.get(i))
        .map(|s| (*s).to_string())
}

fn special(call: &Call) -> Option<FieldFormat> {
    let style = call.int(0, -1)?;
    (0..=3)
        .contains(&style)
        .then_some(FieldFormat::Special { style: style as u8 })
}

fn keystroke(call: &Call) -> Option<FieldFormat> {
    match call.name.as_str() {
        "AFNumber_Keystroke" => number_format(call),
        "AFPercent_Keystroke" => percent_format(call),
        "AFDate_KeystrokeEx" => Some(FieldFormat::Date {
            format: call.text(0)?,
        }),
        "AFDate_Keystroke" => Some(FieldFormat::Date {
            format: table(call, &DATE_FORMATS)?,
        }),
        "AFTime_Keystroke" => Some(FieldFormat::Time {
            format: table(call, &TIME_FORMATS)?,
        }),
        "AFTime_KeystrokeEx" => Some(FieldFormat::Time {
            format: call.text(0)?,
        }),
        "AFSpecial_Keystroke" => special(call),
        "AFSpecial_KeystrokeEx" => {
            let mask = call.text(0)?;
            (!mask.is_empty()).then_some(FieldFormat::Mask { mask })
        }
        _ => None,
    }
}

fn format(call: &Call) -> Option<FieldFormat> {
    match call.name.as_str() {
        "AFNumber_Format" => number_format(call),
        "AFPercent_Format" => percent_format(call),
        "AFDate_FormatEx" => Some(FieldFormat::Date {
            format: call.text(0)?,
        }),
        "AFDate_Format" => Some(FieldFormat::Date {
            format: table(call, &DATE_FORMATS)?,
        }),
        "AFTime_Format" => Some(FieldFormat::Time {
            format: table(call, &TIME_FORMATS)?,
        }),
        "AFTime_FormatEx" => Some(FieldFormat::Time {
            format: call.text(0)?,
        }),
        "AFSpecial_Format" => special(call),
        _ => None,
    }
}

fn validate(call: &Call) -> Option<FieldRange> {
    if call.name != "AFRange_Validate" || call.args.len() != 4 {
        return None;
    }
    let min = call.flag(0, false).then(|| call.arg(1)?.number());
    let max = call.flag(2, false).then(|| call.arg(3)?.number());
    // A bound that is switched on must be a number.
    if min == Some(None) || max == Some(None) {
        return None;
    }
    Some(FieldRange {
        min: min.flatten(),
        max: max.flatten(),
    })
}

fn calculate(call: &Call) -> Option<FieldCalc> {
    if call.name != "AFSimple_Calculate" || call.args.len() != 2 {
        return None;
    }
    let op = call.arg(0)?.string()?.to_ascii_uppercase();
    if !matches!(op.as_str(), "SUM" | "PRD" | "AVG" | "MIN" | "MAX") {
        return None;
    }
    // AFMakeArrayFromList: an array of names, or one comma-separated string.
    let fields: Vec<String> = match call.arg(1)? {
        JsValue::Array(items) => items
            .iter()
            .map(|v| v.string().map(|s| s.trim().to_string()))
            .collect::<Option<_>>()?,
        JsValue::Str(list) => list.split(',').map(|s| s.trim().to_string()).collect(),
        _ => return None,
    };
    let fields: Vec<String> = fields.into_iter().filter(|s| !s.is_empty()).collect();
    (!fields.is_empty()).then_some(FieldCalc { op, fields })
}

/// Parse `source` as a single call with literal arguments.
pub fn parse_call(source: &str) -> Option<Call> {
    let mut p = Parser {
        s: source.as_bytes(),
        i: 0,
    };
    p.skip();
    let name = p.ident()?;
    p.skip();
    p.expect(b'(')?;
    let args = p.list(b')')?;
    p.skip();
    while p.peek() == Some(b';') {
        p.i += 1;
        p.skip();
    }
    (p.i == p.s.len()).then_some(Call { name, args })
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Whitespace and comments.
    fn skip(&mut self) {
        loop {
            match (self.peek(), self.s.get(self.i + 1)) {
                (Some(c), _) if c.is_ascii_whitespace() => self.i += 1,
                (Some(b'/'), Some(b'/')) => {
                    while self.peek().is_some_and(|c| c != b'\n' && c != b'\r') {
                        self.i += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    self.i += 2;
                    while self.i < self.s.len() && !self.s[self.i..].starts_with(b"*/") {
                        self.i += 1;
                    }
                    self.i = (self.i + 2).min(self.s.len());
                }
                _ => return,
            }
        }
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        (self.peek() == Some(c)).then(|| self.i += 1)
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
        {
            self.i += 1;
        }
        let word = &self.s[start..self.i];
        (!word.is_empty() && !word[0].is_ascii_digit())
            .then(|| String::from_utf8_lossy(word).into_owned())
    }

    /// Comma-separated values up to `close`, which is consumed.
    fn list(&mut self, close: u8) -> Option<Vec<JsValue>> {
        let mut out = Vec::new();
        self.skip();
        if self.peek() == Some(close) {
            self.i += 1;
            return Some(out);
        }
        loop {
            self.skip();
            out.push(self.value()?);
            self.skip();
            match self.peek()? {
                b',' => self.i += 1,
                c if c == close => {
                    self.i += 1;
                    return Some(out);
                }
                _ => return None,
            }
        }
    }

    fn value(&mut self) -> Option<JsValue> {
        match self.peek()? {
            b'"' | b'\'' => self.string().map(JsValue::Str),
            b'[' => {
                self.i += 1;
                self.list(b']').map(JsValue::Array)
            }
            b'-' | b'+' | b'.' | b'0'..=b'9' => self.number().map(JsValue::Number),
            _ => {
                let word = self.ident()?;
                match word.as_str() {
                    "true" => Some(JsValue::Bool(true)),
                    "false" => Some(JsValue::Bool(false)),
                    "new" => {
                        self.skip();
                        if self.ident()? != "Array" {
                            return None;
                        }
                        self.skip();
                        self.expect(b'(')?;
                        self.list(b')').map(JsValue::Array)
                    }
                    _ => None,
                }
            }
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.i += 1;
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E'))
        {
            // An exponent may carry its own sign.
            if matches!(self.peek(), Some(b'e' | b'E'))
                && matches!(self.s.get(self.i + 1), Some(b'-' | b'+'))
            {
                self.i += 1;
            }
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out: Vec<u16> = Vec::new();
        let start = self.i;
        // Work on chars so non-ASCII text survives.
        let rest = std::str::from_utf8(&self.s[start..]).ok()?;
        let mut chars = rest.char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                c if c as u32 == quote as u32 => {
                    self.i = start + at + 1;
                    return Some(String::from_utf16_lossy(&out));
                }
                '\\' => {
                    let (_, e) = chars.next()?;
                    match e {
                        'n' => out.push(b'\n' as u16),
                        'r' => out.push(b'\r' as u16),
                        't' => out.push(b'\t' as u16),
                        'u' => {
                            let hex: String =
                                (0..4).filter_map(|_| chars.next()).map(|x| x.1).collect();
                            out.push(u16::from_str_radix(&hex, 16).ok()?);
                        }
                        '\n' => {}
                        other => {
                            let mut buf = [0u16; 2];
                            out.extend_from_slice(other.encode_utf16(&mut buf));
                        }
                    }
                }
                '\n' | '\r' => return None,
                other => {
                    let mut buf = [0u16; 2];
                    out.extend_from_slice(other.encode_utf16(&mut buf));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn behaviour(event: &[u8], source: &str) -> (bool, FieldBehaviour) {
        let mut b = FieldBehaviour::default();
        let ok = b.take(event, source);
        (ok, b)
    }

    #[test]
    fn parses_acrobat_number_format() {
        let (ok, b) = behaviour(b"F", "AFNumber_Format(2, 0, 0, 0, \"\", true);");
        assert!(ok);
        assert_eq!(
            b.format,
            Some(FieldFormat::Number {
                decimals: 2,
                sep_style: 0,
                neg_style: 0,
                currency: String::new(),
                prepend: true
            })
        );
    }

    #[test]
    fn tolerates_whitespace_quotes_comments_and_semicolons() {
        let (ok, b) = behaviour(
            b"K",
            " /* set by Acrobat */\r\n  AFNumber_Keystroke ( 1 ,2,3, 0,'\\u20ac' , false ) ;; // eol\n",
        );
        assert!(ok);
        assert_eq!(
            b.keystroke,
            Some(FieldFormat::Number {
                decimals: 1,
                sep_style: 2,
                neg_style: 3,
                currency: "\u{20ac}".into(),
                prepend: false
            })
        );
    }

    #[test]
    fn resolves_date_and_time_tables() {
        assert_eq!(
            behaviour(b"F", "AFDate_Format(5)").1.format,
            Some(FieldFormat::Date {
                format: "d-mmm-yy".into()
            })
        );
        assert_eq!(
            behaviour(b"K", "AFDate_KeystrokeEx(\"mm/dd/yyyy\");")
                .1
                .keystroke,
            Some(FieldFormat::Date {
                format: "mm/dd/yyyy".into()
            })
        );
        assert_eq!(
            behaviour(b"F", "AFTime_Format(1);").1.format,
            Some(FieldFormat::Time {
                format: "h:MM tt".into()
            })
        );
        assert!(!behaviour(b"F", "AFDate_Format(99)").0);
    }

    #[test]
    fn special_and_mask() {
        assert_eq!(
            behaviour(b"K", "AFSpecial_Keystroke(3);").1.keystroke,
            Some(FieldFormat::Special { style: 3 })
        );
        assert_eq!(
            behaviour(b"K", "AFSpecial_KeystrokeEx(\"999-9999-9\");")
                .1
                .keystroke,
            Some(FieldFormat::Mask {
                mask: "999-9999-9".into()
            })
        );
        assert!(!behaviour(b"F", "AFSpecial_Format(4);").0);
    }

    #[test]
    fn range_validate() {
        assert_eq!(
            behaviour(b"V", "AFRange_Validate(true, 0, true, 1);")
                .1
                .validate,
            Some(FieldRange {
                min: Some(0.0),
                max: Some(1.0)
            })
        );
        assert_eq!(
            behaviour(b"V", "AFRange_Validate(false, 0, true, -2.5)")
                .1
                .validate,
            Some(FieldRange {
                min: None,
                max: Some(-2.5)
            })
        );
    }

    #[test]
    fn simple_calculate_array_and_string_lists() {
        let calc = |src: &str| behaviour(b"C", src).1.calculate;
        assert_eq!(
            calc("AFSimple_Calculate(\"SUM\", new Array (\"a\", \"b.c\"));"),
            Some(FieldCalc {
                op: "SUM".into(),
                fields: vec!["a".into(), "b.c".into()]
            })
        );
        assert_eq!(
            calc("AFSimple_Calculate('avg', 'x, y ,z')"),
            Some(FieldCalc {
                op: "AVG".into(),
                fields: vec!["x".into(), "y".into(), "z".into()]
            })
        );
        assert_eq!(
            calc("AFSimple_Calculate(\"MAX\", [\"p\"])"),
            Some(FieldCalc {
                op: "MAX".into(),
                fields: vec!["p".into()]
            })
        );
        assert_eq!(calc("AFSimple_Calculate(\"MEDIAN\", \"a\")"), None);
    }

    #[test]
    fn rejects_custom_scripts() {
        for src in [
            "DE4_Calc_Line1a();",
            "if (event.value==0) event.value = \"\";",
            "AFNumber_Format(2,0,0,0,\"\",true); app.alert('x')",
            "AFNumber_Format(2,0,0,0,\"\",true",
            "AFNumber_Format(x,0,0,0,\"\",true)",
            "event.value = AFMakeNumber(getField('a').value)",
            "",
        ] {
            for event in [b"K", b"F", b"V", b"C"] {
                assert!(!behaviour(event, src).0, "{src}");
            }
        }
        // A helper under the wrong event is not taken either.
        assert!(!behaviour(b"V", "AFNumber_Format(2,0,0,0,\"\",true)").0);
        assert!(!behaviour(b"Bl", "AFNumber_Format(2,0,0,0,\"\",true)").0);
    }
}
