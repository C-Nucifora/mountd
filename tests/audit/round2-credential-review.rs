// Included after byte-exact copies of the production pure parser functions.
#[cfg(test)]
mod review {
    use super::*;

    fn wrapped(value: &str, crlf: bool) -> String {
        if crlf {
            format!("class: \"inet\"\r\n\tpassword: {value} \r\nattributes: synthetic\r\n")
        } else {
            format!("class: \"inet\"\npassword: {value}\nattributes: synthetic\n")
        }
    }

    #[test]
    fn record_count_compositions() {
        let values = ["\"a1b2\"", "0xc3a9", "\"", "", "0xff"];
        let single = [
            Ok(None),
            Ok(Some("é".to_owned())),
            Err("invalid password metadata"),
            Err("invalid password metadata"),
            Err("invalid password hex"),
        ];
        let mut cases = 0;
        for count in 0..=3u32 {
            for mut code in 0..values.len().pow(count) {
                let mut indices = Vec::new();
                for _ in 0..count {
                    indices.push(code % values.len());
                    code /= values.len();
                }
                for crlf in [false, true] {
                    let mut metadata = "keychain: synthetic\n".to_owned();
                    for &index in &indices {
                        metadata.push_str(&wrapped(values[index], crlf));
                    }
                    let expected = match indices.len() {
                        0 => Err("security password metadata missing"),
                        1 => single[indices[0]].clone(),
                        _ => Err("ambiguous password metadata"),
                    };
                    assert_eq!(parse_password_metadata(&metadata), expected);
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 312);
        println!("record count matrix: {cases} cases");
    }

    #[test]
    fn every_literal_truncation_requires_separate_boundaries() {
        let literal = "\"a1b2c3\"";
        let mut cases = 0;
        for end in 0..=literal.len() {
            for padding in ["", " ", "\t"] {
                let metadata = format!("password: {padding}{}{padding}", &literal[..end]);
                let expected = if end == literal.len() {
                    Ok(None)
                } else {
                    Err("invalid password metadata")
                };
                assert_eq!(parse_password_metadata(&metadata), expected);
                cases += 1;
            }
        }
        for value in ["a1b2c3\"", "\"a1b2c3\"synthetic", "synthetic\"a1b2c3\""] {
            assert_eq!(
                parse_password_metadata(&wrapped(value, false)),
                Err("invalid password metadata")
            );
            cases += 1;
        }
        assert_eq!(cases, 30);
        println!("literal truncation and boundaries: {cases} cases");
    }

    #[test]
    fn unrelated_lines_do_not_hide_or_create_a_record() {
        let noise = [
            "",
            "keychain: synthetic",
            "class: inet",
            "attributes: \"password: synthetic\"",
            "xpassword: synthetic",
            "passwordless: synthetic",
            "note: password: synthetic",
        ];
        assert_eq!(
            parse_password_metadata(&noise.join("\n")),
            Err("security password metadata missing")
        );
        for position in 0..=noise.len() {
            let mut lines = noise.to_vec();
            lines.insert(position, "\tpassword: \"a1b2\"");
            assert_eq!(parse_password_metadata(&lines.join("\r\n")), Ok(None));
        }
        println!("missing record and unique record placement: 9 cases");
    }

    #[test]
    fn encoded_utf8_and_literal_hex_preserve_interpretation() {
        let mut cases = 0;
        for password in ["é", "雪", "🙂", "\t", "\n\n", "é\t🙂\n"] {
            let hex: String = password
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert!(looks_like_hex(&hex));
            for representation in [&hex, &hex.to_uppercase()] {
                for suffix in ["", "  \"...\""] {
                    for crlf in [false, true] {
                        assert_eq!(
                            parse_password_metadata(&wrapped(
                                &format!("0x{representation}{suffix}"),
                                crlf
                            )),
                            Ok(Some(password.to_owned()))
                        );
                        cases += 1;
                    }
                }
            }
            assert_eq!(
                parse_password_metadata(&wrapped(&format!("\"{hex}\""), false)),
                Ok(None)
            );
            cases += 1;
        }
        assert_eq!(cases, 54);
        println!("UTF-8, tabs, repeated newlines and literal hex: {cases} parser cases");
    }

    #[test]
    fn invalid_hex_has_static_diagnostics() {
        let mut cases = 0;
        // Odd length, non-hex, invalid UTF-8 and truncated multibyte sequences.
        for value in [
            "", "a", "123", "gg", "é", "ff", "80", "c3", "c080", "eda080", "f4908080",
        ] {
            for suffix in ["", "  \"synthetic-secret-marker\""] {
                assert_eq!(
                    parse_password_metadata(&wrapped(&format!("0x{value}{suffix}"), false)),
                    Err("invalid password hex")
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 22);
        println!("invalid encoded data with exact static errors: {cases} cases");
    }

    #[test]
    fn restored_old_parser_violates_the_same_oracles() {
        let mut failures_detected = 0;
        for padding in ["", " ", "\t", " \t"] {
            let input = format!("password: {padding}\"{padding}\r\n");
            let expected = Err("invalid password metadata");
            assert_eq!(parse_password_metadata(&input), expected);
            assert_eq!(old::parse(&input), Ok(None));
            assert_ne!(old::parse(&input), expected);
            failures_detected += 1;
        }
        for first in ["\"a1b2\"", "0xc3a9"] {
            for second in ["\"a1b2\"", "0xc3a9", "\"", "", "0xff"] {
                for crlf in [false, true] {
                    let input = format!("{}{}", wrapped(first, crlf), wrapped(second, crlf));
                    let expected = Err("ambiguous password metadata");
                    assert_eq!(parse_password_metadata(&input), expected);
                    assert!(old::parse(&input).is_ok());
                    assert_ne!(old::parse(&input), expected);
                    failures_detected += 1;
                }
            }
        }
        assert_eq!(failures_detected, 24);
        println!("old parser negative control: {failures_detected} oracle violations detected");
    }
}
