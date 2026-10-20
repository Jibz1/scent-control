/// Model letters in a Bluetooth name, from `tools/scentiment_ble.py`.
const MODEL_CODES: [(&str, &str); 7] = [
    ("D", "Diffuser Air 2"),
    ("H", "Hvac"),
    ("P", "Pro"),
    ("M", "Mini"),
    ("T", "Mini 2 LE"),
    ("W", "The Plug-In"),
    ("R", "Diffuser_4"),
];

/// Variant letters in a Bluetooth name, from `tools/scentiment_ble.py`.
const VARIANT_CODES: [(&str, &str); 6] = [
    ("B", "Black"),
    ("C", "Graphite Gold"),
    ("G", "Grey"),
    ("W", "White"),
    ("S", "Silver"),
    ("R", "Red"),
];

/// What a Bluetooth name says about the model. A code we don't know keeps its
/// letter but has no name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameInfo {
    pub model_code: Option<String>,
    pub model: Option<String>,
    pub variant_code: Option<String>,
    pub variant: Option<String>,
}

/// True when a Bluetooth name belongs to a Scentiment diffuser.
pub fn is_scentiment_name(name: &str) -> bool {
    name.to_lowercase().contains("scentiment")
}

/// Reads the model and variant codes from `<prefix>-<ModelCode> <VariantCode> [suffix]`.
pub fn parse_name(name: &str) -> NameInfo {
    let mut tokens = name
        .split_once('-')
        .map_or("", |(_, rest)| rest)
        .split_whitespace();
    let model_code = tokens.next();
    let variant_code = tokens.next();
    NameInfo {
        model_code: model_code.map(str::to_owned),
        model: model_code.and_then(|code| lookup(&MODEL_CODES, code)),
        variant_code: variant_code.map(str::to_owned),
        variant: variant_code.and_then(|code| lookup(&VARIANT_CODES, code)),
    }
}

/// The device ID is the last token of the advertised name, when it is 12 hex
/// digits. Returned in upper case.
pub fn device_id_from_name(name: &str) -> Option<String> {
    let token = name.split_whitespace().next_back()?;
    (token.len() == 12 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| token.to_ascii_uppercase())
}

fn lookup(table: &[(&str, &str)], code: &str) -> Option<String> {
    table
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, name)| (*name).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        model_code: Option<&str>,
        model: Option<&str>,
        variant_code: Option<&str>,
        variant: Option<&str>,
    ) -> NameInfo {
        let own = |s: Option<&str>| s.map(str::to_owned);
        NameInfo {
            model_code: own(model_code),
            model: own(model),
            variant_code: own(variant_code),
            variant: own(variant),
        }
    }

    #[test]
    fn recognises_scentiment_names() {
        assert!(is_scentiment_name("Scentiment-D B 1A2B"));
        assert!(is_scentiment_name("SCENTIMENT-H W"));
        assert!(!is_scentiment_name("Scent K.A5.WIFI"));
        assert!(!is_scentiment_name(""));
    }

    #[test]
    fn reads_model_and_variant_codes() {
        assert_eq!(
            parse_name("Scentiment-D B 1A2B"),
            info(Some("D"), Some("Diffuser Air 2"), Some("B"), Some("Black"))
        );
        assert_eq!(
            parse_name("Scentiment-H W"),
            info(Some("H"), Some("Hvac"), Some("W"), Some("White"))
        );
        assert_eq!(
            parse_name("Scentiment-Z Q"),
            info(Some("Z"), None, Some("Q"), None)
        );
        assert_eq!(parse_name("Scentiment"), info(None, None, None, None));
        assert_eq!(
            parse_name("Scentiment-D"),
            info(Some("D"), Some("Diffuser Air 2"), None, None)
        );
    }

    #[test]
    fn knows_every_model_and_variant_code() {
        let models = [
            ("D", "Diffuser Air 2"),
            ("H", "Hvac"),
            ("P", "Pro"),
            ("M", "Mini"),
            ("T", "Mini 2 LE"),
            ("W", "The Plug-In"),
            ("R", "Diffuser_4"),
        ];
        for (code, name) in models {
            let parsed = parse_name(&format!("Scentiment-{code} B"));
            assert_eq!(parsed.model.as_deref(), Some(name));
        }
        let variants = [
            ("B", "Black"),
            ("C", "Graphite Gold"),
            ("G", "Grey"),
            ("W", "White"),
            ("S", "Silver"),
            ("R", "Red"),
        ];
        for (code, name) in variants {
            let parsed = parse_name(&format!("Scentiment-D {code}"));
            assert_eq!(parsed.variant.as_deref(), Some(name));
        }
    }

    #[test]
    fn device_id_is_the_last_token_when_it_is_twelve_hex_digits() {
        assert_eq!(
            device_id_from_name("Scentiment-D B n A1B2C3D4E5F6").as_deref(),
            Some("A1B2C3D4E5F6")
        );
        assert_eq!(
            device_id_from_name("Scentiment-H B n 0a1b2c3d4e5f").as_deref(),
            Some("0A1B2C3D4E5F")
        );
        for name in [
            "Scentiment-D B",
            "",
            "   ",
            "Scentiment-D B n A1B2C3D4E5F",
            "Scentiment-D B n A1B2C3D4E5F67",
            "Scentiment-D B n G1B2C3D4E5F6",
        ] {
            assert_eq!(device_id_from_name(name), None, "{name:?}");
        }
    }
}
