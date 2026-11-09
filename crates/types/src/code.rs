//! Diagnostic codes (`docs/cli.md`, section 5). A code keeps its meaning forever. `cargo xtask codes`
//! checks that this list equals the tables of that document.

use serde::{Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

macro_rules! codes {
    ($($variant:ident = $text:literal $severity:ident,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Code {
            $($variant,)*
        }

        impl Code {
            pub const ALL: &[Code] = &[$(Code::$variant,)*];

            /// `LC` and four digits.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Code::$variant => $text,)*
                }
            }

            /// A diagnostic takes its severity from its code.
            pub fn severity(self) -> Severity {
                match self {
                    $(Code::$variant => Severity::$severity,)*
                }
            }
        }
    };
}

codes! {
    Lc0001 = "LC0001" Error,
    Lc0002 = "LC0002" Error,
    Lc0003 = "LC0003" Error,
    Lc0004 = "LC0004" Error,
    Lc0005 = "LC0005" Warning,
    Lc0006 = "LC0006" Error,
    Lc0007 = "LC0007" Error,
    Lc0008 = "LC0008" Error,
    Lc0009 = "LC0009" Warning,
    Lc0010 = "LC0010" Error,
    Lc0099 = "LC0099" Error,
    Lc0101 = "LC0101" Error,
    Lc0102 = "LC0102" Error,
    Lc0103 = "LC0103" Error,
    Lc0104 = "LC0104" Error,
    Lc0105 = "LC0105" Error,
    Lc1001 = "LC1001" Error,
    Lc1002 = "LC1002" Error,
    Lc1003 = "LC1003" Error,
    Lc1004 = "LC1004" Error,
    Lc1005 = "LC1005" Error,
    Lc1006 = "LC1006" Error,
    Lc2001 = "LC2001" Error,
    Lc2002 = "LC2002" Error,
    Lc2003 = "LC2003" Error,
    Lc2004 = "LC2004" Error,
    Lc2005 = "LC2005" Error,
    Lc2006 = "LC2006" Error,
    Lc2007 = "LC2007" Error,
    Lc2008 = "LC2008" Error,
    Lc2009 = "LC2009" Error,
    Lc2010 = "LC2010" Error,
    Lc2011 = "LC2011" Error,
    Lc2012 = "LC2012" Error,
    Lc2013 = "LC2013" Error,
    Lc2014 = "LC2014" Error,
    Lc2015 = "LC2015" Error,
    Lc2016 = "LC2016" Warning,
    Lc2017 = "LC2017" Error,
    Lc2018 = "LC2018" Error,
    Lc2019 = "LC2019" Error,
    Lc2020 = "LC2020" Warning,
    Lc2021 = "LC2021" Error,
    Lc2022 = "LC2022" Error,
    Lc2023 = "LC2023" Error,
    Lc2024 = "LC2024" Error,
    Lc2025 = "LC2025" Error,
    Lc2101 = "LC2101" Error,
    Lc2102 = "LC2102" Error,
    Lc2103 = "LC2103" Error,
    Lc2104 = "LC2104" Error,
    Lc2105 = "LC2105" Error,
    Lc2106 = "LC2106" Error,
    Lc2107 = "LC2107" Error,
    Lc3001 = "LC3001" Error,
    Lc3002 = "LC3002" Error,
    Lc3003 = "LC3003" Error,
    Lc3004 = "LC3004" Error,
    Lc3005 = "LC3005" Error,
    Lc3006 = "LC3006" Error,
    Lc3007 = "LC3007" Error,
    Lc3008 = "LC3008" Error,
    Lc3009 = "LC3009" Error,
    Lc3010 = "LC3010" Error,
    Lc3011 = "LC3011" Error,
    Lc3012 = "LC3012" Error,
    Lc3013 = "LC3013" Error,
    Lc3014 = "LC3014" Error,
    Lc3015 = "LC3015" Error,
    Lc3016 = "LC3016" Error,
    Lc3017 = "LC3017" Error,
    Lc3018 = "LC3018" Error,
    Lc3019 = "LC3019" Error,
    Lc3020 = "LC3020" Error,
    Lc3021 = "LC3021" Error,
    Lc3022 = "LC3022" Error,
    Lc4001 = "LC4001" Error,
    Lc4002 = "LC4002" Error,
    Lc4003 = "LC4003" Error,
    Lc4004 = "LC4004" Error,
    Lc4005 = "LC4005" Error,
    Lc4006 = "LC4006" Error,
    Lc4101 = "LC4101" Warning,
    Lc4102 = "LC4102" Warning,
    Lc4103 = "LC4103" Warning,
    Lc4104 = "LC4104" Warning,
    Lc4105 = "LC4105" Warning,
    Lc4106 = "LC4106" Warning,
    Lc5001 = "LC5001" Error,
    Lc5002 = "LC5002" Error,
    Lc5003 = "LC5003" Warning,
    Lc5004 = "LC5004" Warning,
    Lc5005 = "LC5005" Error,
    Lc5006 = "LC5006" Warning,
    Lc5007 = "LC5007" Error,
    Lc5008 = "LC5008" Error,
    Lc5009 = "LC5009" Warning,
    Lc5010 = "LC5010" Error,
    Lc5011 = "LC5011" Error,
    Lc5012 = "LC5012" Error,
    Lc5013 = "LC5013" Error,
    Lc6001 = "LC6001" Error,
    Lc6002 = "LC6002" Warning,
    Lc6003 = "LC6003" Error,
    Lc7002 = "LC7002" Error,
}

impl Code {
    /// A file that gets this code is not read further, and gets no other diagnostic (`docs/cli.md`, section 1).
    pub fn stands_alone(self) -> bool {
        matches!(self, Code::Lc0001 | Code::Lc0003 | Code::Lc0004 | Code::Lc0006 | Code::Lc0007 | Code::Lc0101 | Code::Lc0102)
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_code_is_lc_and_four_digits_once() {
        let mut seen = HashSet::new();
        for code in Code::ALL {
            let text = code.as_str();
            assert!(seen.insert(text), "{text}");
            let digits = text.strip_prefix("LC").unwrap();
            assert_eq!(digits.len(), 4, "{text}");
            assert!(digits.chars().all(|c| c.is_ascii_digit()), "{text}");
        }
    }

    #[test]
    fn a_variant_bears_the_name_of_its_code() {
        for code in Code::ALL {
            assert_eq!(format!("{code:?}").to_uppercase(), code.as_str());
        }
    }

    #[test]
    fn the_codes_that_stand_alone_are_those_of_the_command_line() {
        let alone: Vec<&str> = Code::ALL.iter().filter(|code| code.stands_alone()).map(|code| code.as_str()).collect();
        assert_eq!(alone, ["LC0001", "LC0003", "LC0004", "LC0006", "LC0007", "LC0101", "LC0102"]);
    }

    #[test]
    fn a_code_serializes_as_its_text() {
        assert_eq!(serde_json::to_value(Code::Lc3002).unwrap(), "LC3002");
        assert_eq!(serde_json::to_value(Severity::Warning).unwrap(), "warning");
        assert_eq!(Code::Lc4101.severity(), Severity::Warning);
        assert_eq!(Code::Lc3002.severity().as_str(), "error");
    }
}
